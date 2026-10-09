//! One page host per graph binding (STEP3 §1–§3): the driver thread, the
//! command path that serializes a window's page DTO against the right
//! comparison source and admits it, and the page-mail bridge. Constructed
//! only behind the `TINE_PAGE_HOST` switch (lane 3b flips it); until then
//! the app saves through the old engine.
use super::driver::{Driver, Sink, SystemClock};
use super::production::ProductionIo;
use super::progress::{Clock, Notice, Progress};
use super::*;
use crate::model::Graph;
use crate::{EditKind, FileRev, PageId, Store, Why};
use std::path::Path;
use tine_core::doc::Document;
use tine_core::model::PageDto;

/// The graph binding's page host. Every command carries the window
/// generation and a strictly increasing request id (§3.1).
pub struct PageHost {
    driver: Driver<ProductionIo, SystemClock>,
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
    /// stale window generation, an id out of order, or a stopped host.
    NotAdmitted,
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
    Unchanged,
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
    /// The latest draft write failed.
    pub draft_error: bool,
    /// The conflict has been reported to the window.
    pub conflict_reported: bool,
    /// A trash payload's custody could not be settled.
    pub custody_error: bool,
}

/// `page-mail` (§3.3). The window checks `binding` and `generation` before
/// applying anything.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMail {
    /// The graph binding that sent it.
    pub binding: u64,
    /// The window generation it is for.
    pub generation: u64,
    /// The page key.
    pub key: String,
    /// The page's state, None once the host released it.
    pub page: Option<MailPage>,
    /// The answer to the window's latest request, if any.
    pub answer: Option<MailAnswer>,
    /// Save and draft status.
    pub notice: MailNotice,
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
    /// Refused (unsaved input, a stopped host).
    Refused,
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
}

/// One driver step's results, in host order.
#[derive(Default)]
pub(super) struct Delivery {
    pub events: Vec<Event>,
    /// Published versions' Documents and edit kinds, for the publication
    /// consumer (§5): taken out of the handoff when their bytes match.
    pub documents: Vec<(PageKey, u64, Document)>,
    pub kinds: Vec<(PageKey, Vec<EditKind>)>,
    pub mail: Vec<(PageKey, Mail, MailFacts)>,
}

impl Delivery {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.mail.is_empty()
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
    notices: BTreeMap<PageKey, Notice>,
}

impl Book {
    /// Take this step's results: events (drained once progress has read
    /// them), handoffs routed to answers and publications, and every mail.
    pub fn collect<F: HostIo, C: Clock>(&mut self, progress: &mut Progress<F, C>) -> Delivery {
        progress.host.held = Some(BTreeSet::new());
        let events = progress.take_events();
        let mut delivery = Delivery::default();
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
                        let kinds = self.kinds.entry(page.clone()).or_default();
                        for kind in &handoff.kinds {
                            if !kinds.contains(kind) {
                                kinds.push(*kind);
                            }
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
                    let handoff = self.versions.get_mut(page).and_then(|v| v.remove(version));
                    if let Some(handoff) = handoff.filter(|h| h.bytes == *bytes) {
                        delivery
                            .documents
                            .push((page.clone(), *version, handoff.document));
                    }
                }
                Event::Refused { page, id, reason } => {
                    refused.insert((page.clone(), *id), *reason);
                }
                _ => {}
            }
        }
        let host = &mut progress.host;
        // A request answered for an earlier window generation has no answer
        // event; once applied, nothing names its handoff.
        let applied = host.last_applied;
        self.pending.retain(|(_, id), _| *id > applied);
        let pages = &host.pages;
        let job = host.job.as_ref().map(|job| (&job.page, job.version));
        self.versions.retain(|page, versions| {
            versions.retain(|version, _| {
                pages.get(page).is_some_and(|p| p.version == *version)
                    || job == Some((page, *version))
            });
            !versions.is_empty()
        });
        let keys: Vec<_> = host.outbox.keys().cloned().collect();
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
            .filter(|page| !mailed.contains(*page) && host.pages.contains_key(*page))
            .cloned()
            .collect();
        for page in quiet {
            if self.notices.get(&page) != Some(&progress.notice(&page)) {
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
                    },
                ));
            }
        }
        for (page, _, facts) in &mut delivery.mail {
            facts.notice = progress.notice(page);
            facts.spelling = progress.host.fs.spelling(page);
            self.notices.insert(page.clone(), facts.notice.clone());
        }
        let subscribed = &progress.host.subscriptions;
        self.notices.retain(|page, _| subscribed.contains(page));
        delivery.events = events;
        delivery
    }
}

/// The driver's sink: page mail to the window. Host events go to the
/// publication consumer (§5).
struct Bridge {
    graph: Arc<Graph>,
    binding: u64,
    mail: Box<dyn FnMut(PageMail) + Send>,
}

impl Sink for Bridge {
    fn deliver(&mut self, delivery: Delivery) {
        for (key, mail, facts) in delivery.mail {
            let mail = page_mail(&self.graph, self.binding, key, mail, facts);
            (self.mail)(mail);
        }
    }
}

fn page_mail(graph: &Graph, binding: u64, key: PageKey, mail: Mail, facts: MailFacts) -> PageMail {
    let page = mail.page.map(|page| MailPage {
        version: page.version,
        conflict: page.conflict,
        risk: page.risk,
        disk: page.obs.as_ref().map(DiskToken::of),
        text: match &page.buf {
            _ if !facts.content => MailText::Unchanged,
            None => MailText::NoFile,
            Some(bytes) => match graph.page_dto_for_bytes(&graph.root.join(&facts.spelling), bytes)
            {
                Ok(Some(dto)) => MailText::Page { dto: Box::new(dto) },
                Ok(None) => MailText::Unreadable {
                    message: "not a page path".into(),
                },
                Err(error) => MailText::Unreadable {
                    message: error.to_string(),
                },
            },
        },
    });
    let answer = mail.answer.map(|answer| MailAnswer {
        id: answer.id,
        version: answer.version,
        took: answer.took,
        outcome: match facts.refused {
            Some(reason) => AnswerOutcome::Refused { reason },
            None => AnswerOutcome::Applied,
        },
    });
    PageMail {
        binding,
        generation: mail.generation,
        key,
        page,
        answer,
        notice: MailNotice {
            failures: facts.notice.failures,
            save_error: facts.notice.save_error,
            draft_error: facts.notice.draft_error,
            conflict_reported: facts.notice.conflict_reported,
            custody_error: !facts.notice.custody_error.is_empty(),
        },
    }
}

/// The version a request carries when it was typed on no version the host
/// holds: host versions start at 1, so it is always stale.
const STALE: u64 = 0;

/// A launch that cannot reach a live host (its draft directory keeps
/// failing) is retried this many times before the binding gives up.
const LAUNCH_ATTEMPTS: usize = 3;

impl PageHost {
    /// Bind a page host to `store`'s graph: drafts under
    /// `app_data/drafts-v2/<graph_id>`, recovered draft keys registered
    /// before launch (§2), then the driver. `mail` runs on the driver thread.
    /// The error says why no host could start; the app stays on the old
    /// engine (the switch is off in production until lane 3b).
    pub fn start(
        store: &Store,
        app_data: &Path,
        graph_id: &str,
        binding: u64,
        mail: impl FnMut(PageMail) + Send + 'static,
    ) -> Result<Self, String> {
        let graph = store.graph.clone();
        let trash = crate::model::trash_root(&graph.root).join("pages");
        let io = ProductionIo::new(&graph.root, app_data, graph_id, &trash)
            .map_err(|error| format!("page host drafts: {error}"))?;
        let mut host = Host::new(io, BTreeMap::new());
        host.stop();
        for key in host.recovered_keys() {
            let id = PageId::from(key.as_str());
            let spelling = store.disk_spelling_for_case_alias(&id).unwrap_or(id);
            let lock = graph.page_lock(&graph.root.join(spelling.as_str()));
            host.register(key, spelling.as_str(), lock);
        }
        let bridge = Bridge {
            graph,
            binding,
            mail: Box::new(mail),
        };
        let driver = Driver::spawn(host, SystemClock::new(), bridge);
        let this = Self { driver };
        for _ in 0..LAUNCH_ATTEMPTS {
            this.locked(|host| host.launch());
            if this.driver.shared.state.lock().unwrap().progress.host.alive {
                return Ok(this);
            }
        }
        Err("page host drafts are unavailable".into())
    }

    /// Plan, lock, revalidate on the command thread (§1), then wake the
    /// driver. None once the driver is stopping.
    fn locked<R>(&self, mut step: impl FnMut(&mut Host<ProductionIo>) -> R) -> Option<R> {
        self.driver
            .shared
            .locked_step(|state| state.progress.with_host(&mut step))
    }

    /// The window generation commands must carry.
    pub fn generation(&self) -> u64 {
        self.driver
            .shared
            .state
            .lock()
            .unwrap()
            .progress
            .host
            .generation
    }

    /// A window reload (`window_crash`): unsent input is lost, admitted
    /// requests keep their custody. Returns the new generation.
    pub fn window_reloaded(&self) -> u64 {
        self.driver.shared.with_state(|state| {
            state.progress.with_host(|host| {
                host.window_crash();
                host.generation
            })
        })
    }

    /// A watcher read of `key`'s file (§5): the driver owes the host an
    /// observation of it, taken under its path lock once the page is idle.
    pub fn disk_changed(&self, key: &str) {
        self.driver.shared.with_state(|state| {
            state.observe.entry(key.into()).or_default();
        });
    }

    /// The key naming `page`'s directory entry (§2): a registered key whose
    /// current spelling is the entry's, or the entry's spelling as a new key.
    /// Also whether the host holds that page.
    fn identify(&self, store: &Store, page: &PageId) -> (PageKey, PageId, bool) {
        let spelling = store
            .disk_spelling_for_case_alias(page)
            .unwrap_or_else(|| page.clone());
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let key = host
            .keys
            .iter()
            .find(|key| host.fs.spelling(key) == spelling.as_str())
            .cloned()
            .unwrap_or_else(|| spelling.as_str().into());
        let held = host.pages.contains_key(&key);
        (key, spelling, held)
    }

    fn register(&self, store: &Store, key: &str, spelling: &PageId) {
        let graph = &store.graph;
        let lock = graph.page_lock(&graph.root.join(spelling.as_str()));
        self.driver.shared.with_state(|state| {
            state
                .progress
                .with_host(|host| host.register(key.into(), spelling.as_str(), lock))
        });
    }

    /// Admit `request`; on admission its handoffs wait for its answer.
    fn admit(
        &self,
        request: Request,
        handoffs: Vec<(PageKey, Handoff)>,
    ) -> Result<(), PageRefusal> {
        self.driver.shared.with_state(|state| {
            let id = request.id;
            match state.progress.with_host(|host| host.admit(request)) {
                Disposition::Applied => {
                    for (page, handoff) in handoffs {
                        state.book.pending.insert((page, id), handoff);
                    }
                    Ok(())
                }
                _ => Err(PageRefusal::NotAdmitted),
            }
        })
    }

    /// `page_open` (§2, F4): every editable page, including one with no file
    /// yet, is opened first. With no file the create-only checks run here,
    /// never on its keystrokes. Returns the page's key.
    pub fn open(
        &self,
        store: &Store,
        generation: u64,
        id: u64,
        page: &PageId,
        name: &str,
    ) -> Result<PageKey, PageRefusal> {
        let (key, spelling, held) = self.identify(store, page);
        if !held {
            store
                .transaction(None)
                .page_open_checks(&spelling.file(), name)?;
        }
        self.register(store, &key, &spelling);
        let request = Request {
            id,
            generation,
            page: key.clone(),
            kind: RequestKind::Open,
        };
        self.admit(request, vec![])?;
        Ok(key)
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
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let spelling = if host.keys.contains(key) {
            host.fs.spelling(key)
        } else {
            key.into()
        };
        let page = host.pages.get(key);
        match (resolve, page) {
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
    }

    /// The per-submit checks and serialization (§3.2), before admission.
    fn serialize(
        store: &Store,
        spelling: &str,
        dto: &PageDto,
        source: Option<&Text>,
        kinds: &[EditKind],
    ) -> Result<(Text, Handoff), PageRefusal> {
        let file = PageId::from(spelling).file();
        let tx = store.transaction(None);
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
        store: &Store,
        generation: u64,
        id: u64,
        key: &str,
        dto: &PageDto,
        version: u64,
        resolve: Option<&DiskToken>,
        kinds: &[EditKind],
    ) -> Result<(), PageRefusal> {
        let (spelling, source, base, version) = self.source(key, version, resolve);
        let (bytes, handoff) = Self::serialize(store, &spelling, dto, source.as_ref(), kinds)?;
        let request = Request {
            id,
            generation,
            page: key.into(),
            kind: RequestKind::Submit {
                bytes,
                version,
                resolve: base,
            },
        };
        self.admit(request, vec![(key.into(), handoff)])
    }

    /// `page_move` (§3.1, §8): both endpoints serialized as ordinary submits
    /// on their versions, admitted as the model's two-page move.
    #[allow(clippy::too_many_arguments)]
    pub fn move_blocks(
        &self,
        store: &Store,
        generation: u64,
        id: u64,
        source: (&str, &PageDto, u64),
        receiver: (&str, &PageDto, u64),
        kinds: &[EditKind],
    ) -> Result<(), PageRefusal> {
        let (source_key, source_dto, source_version) = source;
        let (receiver_key, receiver_dto, receiver_version) = receiver;
        let (spelling, old, ..) = self.source(source_key, source_version, None);
        let (source_text, source_handoff) =
            Self::serialize(store, &spelling, source_dto, old.as_ref(), kinds)?;
        let (spelling, old, ..) = self.source(receiver_key, receiver_version, None);
        let (receiver_text, receiver_handoff) =
            Self::serialize(store, &spelling, receiver_dto, old.as_ref(), kinds)?;
        let request = Request {
            id,
            generation,
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
            request,
            vec![
                (source_key.into(), source_handoff),
                (receiver_key.into(), receiver_handoff),
            ],
        )
    }

    /// `page_discard` (§3.1): the window consumed its unsent input first.
    pub fn discard(
        &self,
        generation: u64,
        id: u64,
        key: &str,
        version: u64,
    ) -> Result<(), PageRefusal> {
        let request = Request {
            id,
            generation,
            page: key.into(),
            kind: RequestKind::Discard { version },
        };
        self.admit(request, vec![])
    }

    /// `page_close` (§3.1): the last surface showing the page released it.
    pub fn close(&self, generation: u64, id: u64, key: &str) -> Result<(), PageRefusal> {
        let request = Request {
            id,
            generation,
            page: key.into(),
            kind: RequestKind::Close,
        };
        self.admit(request, vec![])
    }

    /// `page_delete` (§7): the host's delete operation. D4 refuses a page
    /// with unsaved input; Waiting means the page is busy (retry when clean).
    pub fn delete(&self, store: &Store, page: &PageId) -> PageOperation {
        let (key, spelling, _) = self.identify(store, page);
        self.register(store, &key, &spelling);
        match self.locked(|host| host.delete(&key)) {
            Some(Disposition::Applied) => PageOperation::Applied,
            Some(Disposition::Pending) => PageOperation::Pending,
            Some(Disposition::Waiting) => PageOperation::Waiting,
            _ => PageOperation::Refused,
        }
    }

    /// `pages_recoverable` (§4.4): every listed page is clean at or past
    /// that version, or its durable draft holds a version at or past it. A
    /// page the host does not hold has nothing to recover.
    pub fn pages_recoverable(&self, pages: &[(String, u64)]) -> bool {
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let drafts = host.logical_drafts();
        pages
            .iter()
            .all(|(key, version)| match host.pages.get(key) {
                None => true,
                Some(page) => {
                    (page.clean() && page.version >= *version)
                        || drafts
                            .get(key)
                            .is_some_and(|draft| draft.version >= *version)
                }
            })
    }

    /// Stop and join the driver; its last step's mail has been delivered.
    pub fn stop(mut self) {
        self.driver.join();
    }
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
