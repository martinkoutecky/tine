//! Concord (og family 8): sync-copy and VCS-marker discovery, the derived
//! conflict queue, diffs, guarded merges, and recoverable trash. The conflict
//! copy is never treated as a graph page or entered into the cache. Nothing
//! here is persisted: the queue is derived from disk once per open graph and
//! refreshed per published change ([`ConflictQueue`]). A file that cannot be
//! listed, read or diffed is skipped and reported in
//! [`ConflictInventory::unreadable`](tine_core::concord_queue::ConflictInventory::unreadable);
//! it never withholds the healthy conflicts
//! (I-22). Only a failure of the whole store or area scan fails the inventory.

use std::collections::{BTreeMap, HashMap};
use std::io;

use tine_core::concord_queue::{parse_vcs_marker_sides, MarkerConflictDiff};
use tine_core::date::JournalFormat;
use tine_core::doc::{self, Document};
use tine_core::model::{
    decode_page_name, sync_conflict_base, Format, PageDto, PageKind, SyncConflict,
};
use tine_core::projection::{assign_doc_runtime_ids, block_to_dto};
use tine_core::sync_diff::{self, SyncConflictDiff};
use tine_store::{Area, FileId, FileRev, Input, PageHost, PageId, SaveBase, Store};

mod queue;
pub use queue::{conflict_inventory, ConflictQueue};

pub(crate) fn invalid_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid file path")
}

/// A guarded resolve's refusal: `what` changed on disk since the review.
fn changed(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{what} changed on disk"),
    )
}

pub(crate) fn id(store: &Store, rel: &str) -> io::Result<FileId> {
    let config = store.config();
    for (area, dir) in [
        (Area::Pages, &config.pages_dir),
        (Area::Journals, &config.journals_dir),
    ] {
        if let Some(tail) = rel.strip_prefix(&format!("{dir}/")) {
            let file = store.file_id(area, tail).map_err(|_| invalid_path())?;
            return tine_store::is_graph_text(&file)
                .then_some(file)
                .ok_or_else(invalid_path);
        }
    }
    Err(invalid_path())
}

pub(crate) fn read_text(store: &Store, id: &FileId) -> io::Result<(String, FileRev)> {
    crate::parsed_text::read(store, id)
}

pub(crate) fn format(id: &FileId) -> Format {
    Format::from_path(id.as_str().as_ref())
}

pub(crate) fn parse(raw: &str, fmt: Format) -> Document {
    if fmt == Format::Org {
        tine_core::org::parse_org(raw)
    } else {
        doc::parse(raw)
    }
}

pub(crate) fn preview(store: &Store, file: &FileId) -> io::Result<String> {
    let (content, _) = read_text(store, file)?;
    let document = parse(&content, format(file));
    Ok(document
        .pre_block
        .as_deref()
        .or_else(|| document.roots.first().map(|block| block.raw()))
        .map(|raw| {
            raw.lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim()
                .trim_start_matches(['-', '*', ' ', '\t'])
                .chars()
                .take(80)
                .collect()
        })
        .unwrap_or_default())
}

/// One area's listing: every listed file, plus `dir/name: reason` for each
/// entry the scan could not read (reported by the caller, never a reason to
/// drop the rest). A failed scan of the whole area is an error. The one
/// answerer for both conflict listings and the journal listings.
pub(crate) fn area_listing(
    store: &Store,
    area: Area,
) -> io::Result<(Vec<tine_store::FileEntry>, Vec<String>)> {
    let listing = store.scan_area(area, None).map_err(crate::store_error)?;
    let config = store.config();
    let dir = match area {
        Area::Journals => &config.journals_dir,
        _ => &config.pages_dir,
    };
    let unreadable = listing
        .unreadable
        .into_iter()
        .map(|(name, error)| {
            if name.is_empty() {
                format!("{dir}: listing incomplete ({})", error.message)
            } else {
                format!("{dir}/{name}: {}", error.message)
            }
        })
        .collect();
    Ok((listing.files, unreadable))
}

/// Whether `error` is a failure of the whole store rather than of one file: a
/// closed store, which [`crate::store_error`] maps to `BrokenPipe`. Such an
/// error fails the request; every other per-file read, decode, size or parse
/// failure skips that file and is reported (one bad file must not refuse the
/// graph, I-22).
pub(crate) fn store_failed(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::BrokenPipe
}

/// What the conflict walk could not read, kept by source so an incremental
/// refresh replaces exactly the rows it re-derives.
#[derive(Clone, Default)]
struct Unreadable {
    /// Listing entries of both areas from the last whole walk; a journal
    /// refresh replaces the journal rows.
    pages_listing: Vec<String>,
    journals_listing: Vec<String>,
    /// Per-file failures (marker read, copy preview or diff), by path.
    files: BTreeMap<String, String>,
    /// Duplicate-day files whose preview or diff failed, by path.
    journal_days: BTreeMap<String, String>,
}

impl Unreadable {
    /// Record a per-file result: `Ok(Some)` on success, `Ok(None)` for a file
    /// gone since the listing (silent) or one that failed (reported under
    /// `path`), `Err` only for a whole-store failure.
    fn file<T>(&mut self, path: &str, result: io::Result<T>) -> io::Result<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if store_failed(&error) => Err(error),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                self.files.insert(path.to_owned(), error.to_string());
                Ok(None)
            }
        }
    }

    /// The reported rows, once per path, sorted.
    fn rows(&self) -> Vec<String> {
        let mut by_path: BTreeMap<String, String> = BTreeMap::new();
        for (path, reason) in self.journal_days.iter().chain(&self.files) {
            by_path
                .entry(path.clone())
                .or_insert_with(|| format!("{path}: {reason}"));
        }
        let mut rows: Vec<String> = by_path.into_values().collect();
        for row in self.journals_listing.iter().chain(&self.pages_listing) {
            if !rows.contains(row) {
                rows.push(row.clone());
            }
        }
        rows.sort();
        rows
    }
}

/// The page kind, area-relative stem and display name of a page/journal file,
/// or `None` for any other path. One answer for both conflict listings.
fn graph_text_title(store: &Store, file: &FileId) -> Option<(PageKind, String, String)> {
    let config = store.config();
    let (kind, rel) = if let Some(rel) = file
        .as_str()
        .strip_prefix(&format!("{}/", config.journals_dir))
    {
        (PageKind::Journal, rel)
    } else {
        let rel = file
            .as_str()
            .strip_prefix(&format!("{}/", config.pages_dir))?;
        (PageKind::Page, rel)
    };
    if !tine_store::is_graph_text(file) {
        return None;
    }
    let stem = rel.rsplit('/').next()?.rsplit_once('.')?.0.to_owned();
    let title = |stem: &str| {
        if kind == PageKind::Journal {
            let journal_format = JournalFormat::new(
                config.journal_file_name_format.as_deref(),
                config.journal_page_title_format.as_deref(),
            );
            journal_format
                .parse(stem)
                .map(|day| journal_format.title(day))
                .unwrap_or_else(|| stem.to_owned())
        } else {
            decode_page_name(stem, config.file_name_format)
        }
    };
    let name = sync_conflict_base(&stem).map_or_else(|| title(&stem), title);
    Some((kind, stem, name))
}

/// The winner a graph page or journal file shadows when it is a
/// sync-conflict copy; `None` for every other file. Pure path arithmetic.
pub fn sync_copy_of(store: &Store, file: &FileId) -> Option<String> {
    let (_, stem, _) = graph_text_title(store, file)?;
    sync_conflict_base(&stem)?;
    sync_copy_winner(file.as_str())
}

/// The winner a sync-conflict copy shadows: the same directory and extension
/// with the provider's tail removed. `None` when `copy` is not a recognized
/// copy name. Pure path arithmetic; the winner may not exist.
pub fn sync_copy_winner(copy: &str) -> Option<String> {
    let (dir, name) = copy.rsplit_once('/').map_or(("", copy), |(d, n)| (d, n));
    let (stem, ext) = name.rsplit_once('.')?;
    let base = sync_conflict_base(stem)?;
    Some(if dir.is_empty() {
        format!("{base}.{ext}")
    } else {
        format!("{dir}/{base}.{ext}")
    })
}

/// The listing entry for one sync-conflict copy file, or `None` when `file`
/// is not a copy. `winner_exists` answers whether the shadowed file is on
/// disk. Cost O(copy preview bytes).
fn sync_copy_entry(
    store: &Store,
    file: &FileId,
    winner_exists: impl FnOnce(&str) -> io::Result<bool>,
) -> io::Result<Option<SyncConflict>> {
    let Some((kind, stem, base_name)) = graph_text_title(store, file) else {
        return Ok(None);
    };
    let Some(base_stem) = sync_conflict_base(&stem) else {
        return Ok(None);
    };
    let Some(winner) = sync_copy_winner(file.as_str()) else {
        return Ok(None);
    };
    let tag = stem[base_stem.len()..]
        .trim_matches(|ch: char| matches!(ch, '.' | ' ' | '(' | ')'))
        .to_owned();
    Ok(Some(SyncConflict {
        path: file.as_str().to_owned(),
        base_name,
        base_path: winner_exists(&winner)?.then_some(winner),
        kind,
        tag,
        preview: preview(store, file)?,
    }))
}

fn sort_copies(out: &mut [SyncConflict]) {
    out.sort_by(|a, b| {
        a.base_name
            .cmp(&b.base_name)
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// List Syncthing, Dropbox and Seafile copies in pages and journals. A copy
/// that cannot be read is skipped and reported; a failed area scan or a closed
/// store is an error and callers keep their last good list. Cost O(P + J +
/// conflict-copy bytes), including winner presence checks.
pub fn list_sync_conflicts(store: &Store) -> io::Result<Vec<SyncConflict>> {
    Ok(sync_copies(store, &mut Unreadable::default())?)
}

fn sync_copies(store: &Store, bad: &mut Unreadable) -> io::Result<Vec<SyncConflict>> {
    let mut out = Vec::new();
    for area in [Area::Journals, Area::Pages] {
        let (files, unreadable) = area_listing(store, area)?;
        match area {
            Area::Journals => bad.journals_listing = unreadable,
            _ => bad.pages_listing = unreadable,
        }
        let present: std::collections::HashSet<_> = files
            .iter()
            .map(|entry| entry.id.as_str().to_owned())
            .collect();
        for entry in &files {
            let entry_result =
                sync_copy_entry(store, &entry.id, |winner| Ok(present.contains(winner)));
            out.extend(bad.file(entry.id.as_str(), entry_result)?.flatten());
        }
    }
    sort_copies(&mut out);
    Ok(out)
}

/// The ancestor a sync-copy 3-way review uses: the newest candidate that
/// differs from the winner's current bytes (master's rule: a base identical
/// to the winner is almost always the admission artifact, the winner's
/// post-sync bytes recorded before this diff ran, and 3-way against it would
/// blanket-suggest "theirs"). When that candidate equals the copy's current
/// bytes the review stays 2-way (Tine addition): on og this is normally the
/// copy's own artifact — this device's last save, which Syncthing renamed to
/// the copy when the other device's edit won the winner name — and 3-way
/// against it would pre-select discarding this device's edit everywhere. A
/// copy that genuinely equals the ancestor cannot be told apart from it, and
/// an older base could turn a winner-side revert into a "theirs" suggestion,
/// so neither side is pre-selected. Returns the base and its identity token
/// (sha256 hex of its bytes).
fn pick_base<'a>(candidates: &'a [String], mine: &str, theirs: &str) -> Option<(&'a str, String)> {
    use sha2::{Digest, Sha256};
    let base = candidates.iter().find(|base| base.as_str() != mine)?;
    (base.as_str() != theirs).then(|| {
        (
            base.as_str(),
            format!("{:x}", Sha256::digest(base.as_bytes())),
        )
    })
}

/// Structural diff of two exact files. `bases` are candidate common
/// ancestors, newest first (the Concord base ledger: the copy's pin, then the
/// winner's retained revisions); with one that differs from the winner the
/// diff is 3-way and its rows carry suggestions the UI pre-selects (never
/// applies), stamped with `merge_base_rev`. With none it is the 2-way diff.
/// A missing file or invalid path returns `None`; undecodable bytes error as
/// in v0.6.5. Cost O(both file bytes + base bytes + blocks).
pub fn sync_conflict_diff(
    store: &Store,
    winner: &str,
    conflict: &str,
    bases: &[String],
) -> io::Result<Option<SyncConflictDiff>> {
    let (Ok(win), Ok(conf)) = (id(store, winner), id(store, conflict)) else {
        return Ok(None);
    };
    let (mine, base_rev) = match read_text(store, &win) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let (theirs, conflict_rev) = match read_text(store, &conf) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let fmt = format(&win);
    let (mine_doc, theirs_doc) = (parse(&mine, fmt), parse(&theirs, format(&conf)));
    let mut diff = match pick_base(bases, &mine, &theirs) {
        Some((base, token)) => {
            let mut diff = sync_diff::diff3_docs(&parse(base, fmt), &mine_doc, &theirs_doc);
            diff.merge_base_rev = Some(token);
            diff
        }
        None => sync_diff::diff_docs(&mine_doc, &theirs_doc),
    };
    diff.base_rev = base_rev.into();
    diff.conflict_rev = conflict_rev.into();
    Ok(Some(diff))
}

/// What the parser says about one pre-block text (I-12, D06): its accepted
/// property lines and its literal containers. Markdown properties are the
/// parser's primary `key:: value` lines; Org properties are `#+KEY: value`
/// directives. Content inside a literal container (fence, `#+BEGIN_SRC`, quote,
/// ...) is never a property. Parsed once per text. A parser-refused text
/// (quarantined) exposes nothing, so every line is plain text and kept as such.
pub(crate) struct PreRegions {
    /// Byte offset of a property's line start -> (lowercase key, trimmed value).
    props: HashMap<usize, (String, String)>,
    /// Literal containers as (first line start, end) byte ranges.
    containers: Vec<(usize, usize)>,
}

pub(crate) fn pre_regions(text: &str, fmt: Format) -> PreRegions {
    let org = fmt == Format::Org;
    let regions = tine_core::block_regions::parse_document(text, org);
    let mut found = PreRegions {
        props: HashMap::new(),
        containers: Vec::new(),
    };
    if regions.quarantined {
        return found;
    }
    for p in &regions.properties {
        if if org { p.directive } else { p.primary } {
            found
                .props
                .insert(p.line.0, (p.key.to_ascii_lowercase(), p.value.clone()));
        }
    }
    for block in &regions.literal_blocks {
        let start = text[..block.range.0].rfind('\n').map_or(0, |at| at + 1);
        found
            .containers
            .push((start, block.range.1.min(text.len())));
    }
    found
}

impl PreRegions {
    /// The container whose lines include the line `start..end`, if any.
    pub(crate) fn container_of(&self, start: usize, end: usize) -> Option<usize> {
        self.containers
            .iter()
            .position(|&(from, to)| start < to && end > from)
    }
    pub(crate) fn property(&self, line_start: usize) -> Option<&(String, String)> {
        self.props.get(&line_start)
    }
}

/// A container compared as one unit: line endings and trailing blank lines are
/// not content.
fn container_text(text: &str, (from, to): (usize, usize)) -> String {
    text[from..to].replace('\r', "").trim_end().to_owned()
}

/// `a, b, c` without the join method (the client path guard scans for it).
fn comma_list(items: &[String]) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(item);
    }
    out
}

/// "Both (merge)" for the page's own pre-block: `mine`, plus what only
/// `theirs` holds — property lines with new keys (kept among the leading
/// property lines), members of a `tags`/`alias`/`aliases` list, and free-text
/// lines — so the conflict copy's pre-block survives the merge (C3W W5, L03;
/// master drops the free text and the values). A key both sides set to
/// different values, or an Org drawer line only `theirs` has, cannot be kept
/// twice: the merge refuses and names it so the user picks mine or theirs
/// (scenario: honest multi-device divergence of one page property).
fn union_pre(mine: Option<&str>, theirs: Option<&str>, fmt: Format) -> io::Result<Option<String>> {
    let mine = mine.unwrap_or("");
    let Some(theirs) = theirs else {
        return Ok((!mine.is_empty()).then(|| mine.to_owned()));
    };
    let mine_regions = pre_regions(mine, fmt);
    let their_regions = pre_regions(theirs, fmt);
    let mine_lines: Vec<(usize, &str)> = {
        let mut at = 0;
        mine.split_inclusive('\n')
            .map(|line| {
                at += line.len();
                (at - line.len(), line)
            })
            .collect()
    };
    let body = |l: &str| l.trim_end_matches(['\r', '\n']).to_owned();
    let mut mine_props: HashMap<String, (usize, &str)> = HashMap::new();
    let mut mine_text: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, &(start, line)) in mine_lines.iter().enumerate() {
        if mine_regions
            .container_of(start, start + line.len())
            .is_some()
        {
            continue;
        }
        mine_text.insert(body(line).trim().to_owned());
        if let Some((key, value)) = mine_regions.property(start) {
            mine_props.insert(key.clone(), (i, value.as_str()));
        }
    }
    let mine_containers: std::collections::HashSet<String> = mine_regions
        .containers
        .iter()
        .map(|&c| container_text(mine, c))
        .collect();
    let mut extra_props = Vec::new();
    let mut extra_text = Vec::new();
    let mut members: HashMap<usize, Vec<String>> = HashMap::new();
    let mut clashes = Vec::new();
    let mut seen_containers = std::collections::HashSet::new();
    let mut at = 0;
    for raw_line in theirs.split_inclusive('\n') {
        let (start, end) = (at, at + raw_line.len());
        at = end;
        let line = raw_line.trim_end_matches(['\r', '\n']);
        // A literal container is one atomic unit: identical to one of mine's it
        // is dropped, otherwise it is kept whole after mine's text, never
        // line-merged with mine's lines or properties.
        if let Some(c) = their_regions.container_of(start, end) {
            if seen_containers.insert(c) {
                let unit = container_text(theirs, their_regions.containers[c]);
                if !mine_containers.contains(&unit) {
                    extra_text.push(unit);
                }
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || mine_text.contains(trimmed) {
            continue;
        }
        match their_regions.property(start) {
            Some((key, value)) => match mine_props.get(key) {
                None => extra_props.push(line.to_owned()),
                Some((_, mine_value)) if mine_value.trim() == value.trim() => {}
                Some((at, mine_value)) if matches!(key.as_str(), "tags" | "alias" | "aliases") => {
                    let split = |v: &str| -> Vec<String> {
                        v.split(tine_core::refs::is_linkable_property_separator)
                            .map(|m| m.trim().to_owned())
                            .filter(|m| !m.is_empty())
                            .collect()
                    };
                    let held: Vec<String> = split(mine_value)
                        .iter()
                        .chain(members.get(at).into_iter().flatten())
                        .map(|m| tine_core::refs::normalize(m))
                        .collect();
                    let new: Vec<String> = split(value)
                        .into_iter()
                        .filter(|m| !held.contains(&tine_core::refs::normalize(m)))
                        .collect();
                    members.entry(*at).or_default().extend(new);
                }
                Some(_) => clashes.push(key.clone()),
            },
            None if fmt == Format::Org && trimmed.starts_with(':') => {
                clashes.push("properties drawer".to_owned())
            }
            None => extra_text.push(line.to_owned()),
        }
    }
    if !clashes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the page's own {} differs between the two versions; keep mine or theirs for the page properties",
                comma_list(&clashes)
            ),
        ));
    }
    if extra_props.is_empty() && extra_text.is_empty() && members.is_empty() {
        return Ok((!mine.is_empty()).then(|| mine.to_owned()));
    }
    let lead = mine_lines
        .iter()
        .take_while(|&&(start, _)| mine_regions.property(start).is_some())
        .count();
    let mut out = String::with_capacity(mine.len() + theirs.len());
    let push_line = |out: &mut String, line: &str| {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(line);
    };
    for (i, &(_, line)) in mine_lines.iter().enumerate() {
        if i == lead {
            for prop in &extra_props {
                push_line(&mut out, prop);
                out.push('\n');
            }
        }
        match members.get(&i) {
            Some(new) if !new.is_empty() => {
                let content = line.trim_end_matches(['\r', '\n']);
                push_line(&mut out, content.trim_end());
                out.push_str(", ");
                out.push_str(&comma_list(new));
                out.push_str(&line[content.len()..]);
            }
            _ => push_line(&mut out, line),
        }
    }
    if lead == mine_lines.len() {
        for prop in &extra_props {
            push_line(&mut out, prop);
        }
    }
    for text in &extra_text {
        push_line(&mut out, text);
    }
    if mine.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(Some(out))
}

pub(crate) fn choose_pre(
    choice: &str,
    fmt: Format,
    mine: &Document,
    theirs: &Document,
) -> io::Result<Option<String>> {
    Ok(match choice {
        "theirs" => theirs.pre_block.clone(),
        "mine" => mine.pre_block.clone(),
        _ => union_pre(mine.pre_block.as_deref(), theirs.pre_block.as_deref(), fmt)?,
    })
}

pub(crate) fn merge_refused(refusal: sync_diff::MergeRefused) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, refusal.to_string())
}

pub(crate) fn dto(store: &Store, id: &PageId, mut doc: Document) -> PageDto {
    assign_doc_runtime_ids(&mut doc.roots, id.as_str());
    let config = store.config();
    let stem = id
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or(id.as_str())
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or("");
    let kind = if id
        .as_str()
        .starts_with(&format!("{}/", config.journals_dir))
    {
        PageKind::Journal
    } else {
        PageKind::Page
    };
    let name = if kind == PageKind::Journal {
        let fmt = JournalFormat::new(
            config.journal_file_name_format.as_deref(),
            config.journal_page_title_format.as_deref(),
        );
        fmt.parse(stem)
            .map(|day| fmt.title(day))
            .unwrap_or_else(|| stem.to_owned())
    } else {
        decode_page_name(stem, config.file_name_format)
    };
    PageDto {
        title: name.clone(),
        name,
        kind,
        pre_block: doc.pre_block,
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
        format: format(&id.file()),
        read_only: false,

        guide: false,
    }
}

/// Merge the selected blocks into the winner, then trash the copy in one
/// guarded transaction. Stale UI revisions yield `winner changed on disk` or
/// `conflict copy changed on disk`; Org round-trip refusal is unchanged.
/// `merge_base_rev` is the diff's base token: `None` resolves 2-way (a
/// `"merged"` decision then refuses). With `Some` and a `"merged"` decision
/// the SAME base is re-derived from `bases`; when it is gone or different the
/// resolve refuses with `merge base changed since the review`, so a merged
/// body is only ever computed from the three texts the user saw. Other
/// decisions never read the base. Cost O(both file bytes + blocks) per attempt, at most
/// four attempts.
pub fn resolve_sync_conflict(
    store: &Store,
    host: Option<&PageHost>,
    winner: &str,
    conflict: &str,
    decisions: &HashMap<String, String>,
    base_rev: &str,
    conflict_rev: &str,
    merge_base_rev: Option<&str>,
    bases: &[String],
    pre_choice: &str,
) -> io::Result<()> {
    let win = id(store, winner)?;
    let conf = id(store, conflict)?;
    if win == conf {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "winner and conflict are the same file",
        ));
    }
    // A copy is merged only into the page it shadows. Scenario: sync-service
    // delivery of a copy whose base page does not exist (or a stale caller
    // pairing) must never merge its content into another page and trash it
    // (C3 L01; storage-contract refusal table).
    if sync_copy_winner(conf.as_str()).as_deref() != Some(win.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the conflict copy does not shadow this page; not merging",
        ));
    }
    fold_pair(
        store,
        host,
        (&win, &conf),
        decisions,
        (base_rev, conflict_rev),
        merge_base_rev,
        bases,
        pre_choice,
    )
}

/// The guarded two-file fold shared by a sync copy and a duplicate journal
/// day, once the caller has proved the pairing: `conf` is merged into `win`
/// per the row decisions and trashed recoverably. Unsaved input in either
/// page is refused as a change on disk would be, so the fold never drops it.
///
/// With a host, one host operation (DESIGN §3.1) planned from the bytes the
/// fold read: the winner's write, and for a duplicate day the stray's
/// deletion as a move's source (the winner's new bytes hold the stray's
/// rows), so a crash between them completes at relaunch rather than leaving
/// the stray to fold in again. A sync copy is no page (`Store::as_page`), so
/// it is trashed after the operation applied, guarded by the bytes the fold
/// read (DESIGN §3.4, post): a copy changed since is kept. With no host, one
/// transaction under the retained-writer route, as before P3.
#[allow(clippy::too_many_arguments)]
fn fold_pair(
    store: &Store,
    host: Option<&PageHost>,
    (win, conf): (&FileId, &FileId),
    decisions: &HashMap<String, String>,
    revs: (&str, &str),
    merge_base_rev: Option<&str>,
    bases: &[String],
    pre_choice: &str,
) -> io::Result<()> {
    const EXHAUSTED: &str = "conflict files changed repeatedly during merge";
    let page = store.as_page(win).ok_or_else(invalid_path)?;
    let copy = store.as_page(conf);
    let refused = |key: &str| match copy.as_ref() {
        Some(copy) if copy.as_str().eq_ignore_ascii_case(key) => changed("conflict copy"),
        _ => changed("winner"),
    };
    let fold = || {
        let pair = (win, conf);
        folded(
            store,
            &page,
            pair,
            decisions,
            revs,
            merge_base_rev,
            bases,
            pre_choice,
        )
    };
    let transaction = |fold: &Folded, include_source: bool| {
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[
                tine_store::EditKind::ReplacePage,
                tine_store::EditKind::DeletePage,
            ],
            &page,
            SaveBase::Existing(fold.win_rev.clone()),
            &fold.doc,
        );
        if include_source {
            tx.trash(conf, fold.conf_rev.clone(), tine_store::TrashIf::Any);
        }
        tx
    };
    let Some(running) = host else {
        let discover = || Ok(crate::retained::pages(store, [win, conf]));
        return crate::retained::reserved(host, Input::Refuse, discover, refused, |_| {
            crate::retry_on_conflict(EXHAUSTED, || {
                let fold = fold()?;
                Ok(crate::commit_retry(transaction(&fold, true).commit())?.then_some(()))
            })
        });
    };
    let mut copy_rev = None;
    let plan = || {
        let fold = fold()?;
        if copy.is_none() {
            copy_rev = Some(fold.conf_rev.clone());
        }
        let mut op = crate::retained::plan(transaction(&fold, copy.is_some()))?;
        if let Some(source) = &copy {
            op.moved = Some((page.clone(), source.clone()));
        }
        Ok(Some(op))
    };
    let touched = crate::retained::pages(store, [win, conf]);
    let applied = crate::retained::operation(
        running,
        Input::Refuse,
        &touched,
        "the merge",
        EXHAUSTED,
        refused,
        plan,
    );
    // Set by a plan only for a sync copy, which is no page of the operation.
    let Some(rev) = copy_rev else {
        return applied;
    };
    applied.map_err(|error| match error.kind() {
        // `Uncertain`: the merge may still complete; the copy stays.
        io::ErrorKind::TimedOut => io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{error} The conflict copy was kept."),
        ),
        _ => error,
    })?;
    // Scenario: sync-service delivery changed the copy after the review.
    let kept = || {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the conflict copy changed on disk and was kept",
        )
    };
    trash_copy(store, conf, rev)
        .and_then(|trashed| trashed.ok_or_else(kept))
        .map_err(|error| crate::after_partial("The merge was saved", error))
}

/// The pair's fold as the review decided it, from the texts read now.
struct Folded {
    doc: PageDto,
    win_rev: FileRev,
    conf_rev: FileRev,
}

#[allow(clippy::too_many_arguments)]
fn folded(
    store: &Store,
    page: &PageId,
    (win, conf): (&FileId, &FileId),
    decisions: &HashMap<String, String>,
    (base_rev, conflict_rev): (&str, &str),
    merge_base_rev: Option<&str>,
    bases: &[String],
    pre_choice: &str,
) -> io::Result<Folded> {
    let (mine, win_rev) = read_text(store, win)?;
    let (theirs, conf_rev) = read_text(store, conf)?;
    if String::from(win_rev.clone()) != base_rev {
        return Err(changed("winner"));
    }
    if String::from(conf_rev.clone()) != conflict_rev {
        return Err(changed("conflict copy"));
    }
    let fmt = format(win);
    if fmt == Format::Org
        && (!tine_core::org::org_editable(&mine) || !tine_core::org::org_editable(&theirs))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "an org file in this pair does not round-trip; not merging",
        ));
    }
    let mine_doc = parse(&mine, fmt);
    let their_doc = parse(&theirs, format(conf));
    // Only a `"merged"` row reads the base (mine/theirs/both do not), so a
    // base that is gone or different blocks nothing else: a ledger read
    // failure never refuses a resolve. For a merged row, scenario:
    // sync-service delivery or an honest concurrent instance moved the
    // ledger between review and apply; the user must review the body the
    // merge would now compute.
    let wants_merged = decisions.values().any(|d| d == "merged");
    let base_doc = match (merge_base_rev, wants_merged) {
        (Some(token), true) => match pick_base(bases, &mine, &theirs) {
            Some((base, current)) if current == token => Some(parse(base, fmt)),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "merge base changed since the review",
                ))
            }
        },
        _ => None,
    };
    // A forged `"merged"` decision on a 2-way review refuses the whole
    // resolve (malformed input).
    let roots = sync_diff::merge_blocks3(
        base_doc.as_ref().map(|doc| doc.roots.as_slice()),
        &mine_doc.roots,
        &their_doc.roots,
        None,
        decisions,
    )
    .map_err(merge_refused)?;
    let pre_block = choose_pre(pre_choice, fmt, &mine_doc, &their_doc)?;
    let doc = dto(store, page, Document { pre_block, roots });
    Ok(Folded {
        doc,
        win_rev,
        conf_rev,
    })
}

/// Trash a sync-conflict copy over `rev`, the bytes last read: no page
/// (`Store::as_page` names none: `tests/host_op_conflicts.rs`
/// `a_sync_copy_is_never_a_page`), so no host operation. `None`: the copy
/// changed since.
fn trash_copy(store: &Store, copy: &FileId, rev: FileRev) -> io::Result<Option<()>> {
    let mut tx = store.transaction(None);
    tx.trash(copy, rev, tine_store::TrashIf::Any);
    Ok(crate::commit_retry(tx.commit())?.then_some(()))
}

/// Recoverably trash only a sync copy, with four revision-guard retries. The
/// copy is no page, so neither a host operation nor a reservation fences
/// it. Cost O(copy bytes) per attempt.
pub fn trash_sync_conflict(
    store: &Store,
    _host: Option<&PageHost>,
    conflict: &str,
) -> io::Result<()> {
    let conf = id(store, conflict)?;
    let stem = conf
        .as_str()
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(stem, _)| stem);
    if !stem.is_some_and(|stem| sync_conflict_base(stem).is_some()) {
        return Err(invalid_path());
    }
    crate::retry_on_conflict("conflict copy changed repeatedly during trash", || {
        let max = Some(tine_store::PARSE_INPUT_MAX_BYTES);
        let rev = match store.read(&conf, max) {
            Ok((_, rev)) => rev,
            Err(tine_store::StoreError::NotFound) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no such conflict file",
                ))
            }
            Err(error) => return Err(crate::store_error(error)),
        };
        trash_copy(store, &conf, rev)
    })
}

/// Two-way diff of a duplicate journal day's canonical file against one stray
/// (master 9dc54e4a7). The two files have no common ancestor (two files that
/// came to claim one day, not one document that diverged), so the diff is
/// always 2-way. A cross-format `.md`/`.org` pair cannot be folded and returns
/// `None`, which the queue shows as file rows without row choices. Cost
/// O(both file bytes + blocks).
pub fn duplicate_journal_diff(
    store: &Store,
    canonical: &str,
    stray: &str,
) -> io::Result<Option<SyncConflictDiff>> {
    let (Ok(keep), Ok(other)) = (id(store, canonical), id(store, stray)) else {
        return Ok(None);
    };
    if format(&keep) != format(&other) {
        return Ok(None);
    }
    sync_conflict_diff(store, canonical, stray, &[])
}

/// Fold one stray of a duplicate journal day into the day's canonical file per
/// the reviewed row decisions and trash the stray recoverably, through the
/// same guarded fold as a sync copy (revision guards, org round-trip firewall,
/// one transaction). Refusals, each with its scenario: the two paths are not
/// files of ONE duplicate day, or `canonical` is not that day's keeper (a stale
/// review after sync-service delivery, an external rename or a journal-format
/// change re-sorted the day; never a merge of two unrelated pages); a
/// cross-format pair (malformed pairing: an Org body would be rewritten as
/// Markdown or the reverse). Cost O(journal listing + both file bytes).
#[allow(clippy::too_many_arguments)]
pub fn resolve_duplicate_journal_day(
    store: &Store,
    host: Option<&PageHost>,
    canonical: &str,
    stray: &str,
    decisions: &HashMap<String, String>,
    base_rev: &str,
    stray_rev: &str,
    pre_choice: &str,
) -> io::Result<()> {
    let refuse = |kind, why| Err(io::Error::new(kind, why));
    if canonical == stray {
        return refuse(
            io::ErrorKind::InvalidInput,
            "canonical and stray are the same file",
        );
    }
    let Some(day) = crate::journals::journal_conflicts(store)?
        .into_iter()
        .find(|day| day.files.iter().any(|file| file.path == canonical))
    else {
        return refuse(
            io::ErrorKind::NotFound,
            "not a file of a duplicate journal day",
        );
    };
    if !day.files.iter().any(|file| file.path == stray) {
        return refuse(
            io::ErrorKind::InvalidInput,
            "the two files are not the same journal day",
        );
    }
    // `journal_conflicts` sorts canonical-first: files[0] is the keeper.
    if day.files.first().map(|file| file.path.as_str()) != Some(canonical) {
        return refuse(
            io::ErrorKind::InvalidInput,
            "that file is not the day's canonical file",
        );
    }
    let (keep, other) = (id(store, canonical)?, id(store, stray)?);
    if format(&keep) != format(&other) {
        return refuse(
            io::ErrorKind::InvalidInput,
            "a Markdown and an Org file of one day cannot be merged",
        );
    }
    fold_pair(
        store,
        host,
        (&keep, &other),
        decisions,
        (base_rev, stray_rev),
        None,
        &[],
        pre_choice,
    )
}

/// Block diff of a marker-bearing page's own sides: 3-way against the diff3
/// base the markers carry (with Fossil's suggestion as the artifact), else
/// 2-way. Read-only. Both revs are the whole marker file's rev, so the resolve
/// guard rejects decisions made against a version the VCS has since changed.
/// `Ok(None)` if the path is invalid, gone, or not conflicted. Cost O(file
/// bytes + blocks).
pub fn vcs_marker_conflict_diff(
    store: &Store,
    rel: &str,
) -> io::Result<Option<MarkerConflictDiff>> {
    let Ok(file) = id(store, rel) else {
        return Ok(None);
    };
    let (content, rev) = match read_text(store, &file) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(sides) = parse_vcs_marker_sides(&content, format(&file)) else {
        return Ok(None);
    };
    let fmt = format(&file);
    let mine = parse(&sides.mine, fmt);
    let theirs = parse(&sides.theirs, fmt);
    let mut diff = match sides.base.as_deref() {
        Some(base) => {
            let artifact = sides.suggested.as_deref().map(|text| parse(text, fmt));
            sync_diff::diff3_docs_with_artifact(
                &parse(base, fmt),
                &mine,
                &theirs,
                artifact.as_ref(),
            )
        }
        // No ancestor: no `BothChanged` verdict, so no proposal of either kind.
        None => sync_diff::diff_docs(&mine, &theirs),
    };
    let rev: String = rev.into();
    diff.base_rev = rev.clone();
    diff.conflict_rev = rev;
    Ok(Some(MarkerConflictDiff {
        mine_label: sides.mine_label,
        theirs_label: sides.theirs_label,
        regions: sides.regions,
        diff,
    }))
}

/// Apply the user's per-row decisions to a marker-bearing page and write the
/// clean result: the one write R-VCS-MARKERS permits to such a file, through
/// `SaveBase::ResolvingMarkers`, which stages the pre-resolution bytes in
/// conflict trash in the same transaction. A stale `base_rev` returns
/// `AlreadyExists` ("file changed on disk") and writes nothing. The merge
/// re-derives the SAME alignment (and any merged body) from the guarded bytes,
/// never from the client. Cost O(file bytes + blocks) per attempt, at most
/// four attempts.
pub fn resolve_vcs_marker_conflict(
    store: &Store,
    host: Option<&PageHost>,
    rel: &str,
    decisions: &HashMap<String, String>,
    base_rev: &str,
    pre_choice: &str,
) -> io::Result<()> {
    const EXHAUSTED: &str = "marker file changed repeatedly during resolve";
    let file = id(store, rel)?;
    let page = store.as_page(&file).ok_or_else(invalid_path)?;
    // A retained writer (STEP3 §7): unsaved input on the page is refused as
    // a change on disk would be.
    let refused = |_: &str| changed("file");
    // The review's text, read now. Scenario: the VCS or an external editor
    // changed the file after the review was computed; the user must review
    // the new version.
    let reviewed = || {
        let (content, rev) = read_text(store, &file)?;
        if String::from(rev.clone()) != base_rev {
            return Err(changed("file"));
        }
        let merged = marker_resolution(store, &file, &page, &content, decisions, pre_choice)?;
        Ok((content, rev, merged))
    };
    let transaction = |rev, merged: &PageDto| {
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[tine_store::EditKind::ReplacePage],
            &page,
            SaveBase::ResolvingMarkers(rev),
            merged,
        );
        tx
    };
    let Some(running) = host else {
        let discover = || Ok(vec![page.clone()]);
        return crate::retained::reserved(None, Input::Refuse, discover, refused, |_| {
            crate::retry_on_conflict(EXHAUSTED, || {
                let (_, rev, merged) = reviewed()?;
                Ok(crate::commit_retry(transaction(rev, &merged).commit())?.then_some(()))
            })
        });
    };
    // C4: the durable non-page marker copy precedes the read-only planner.
    let (content, rev, merged) = reviewed()?;
    transaction(rev, &merged).plan().map_err(crate::why_error)?;
    store
        .stage_marker_copy(&file, content.as_bytes())
        .map_err(crate::why_error)?;
    let plan = || {
        let (_, rev, merged) = reviewed()?;
        crate::retained::plan(transaction(rev, &merged)).map(Some)
    };
    crate::retained::operation(
        running,
        Input::Refuse,
        std::slice::from_ref(&page),
        "resolving the merge",
        EXHAUSTED,
        refused,
        plan,
    )
}

/// The page a marker resolution writes: the review's `decisions` over the
/// marker sides of `content`, with the chosen pre-block.
fn marker_resolution(
    store: &Store,
    file: &FileId,
    page: &PageId,
    content: &str,
    decisions: &HashMap<String, String>,
    pre_choice: &str,
) -> io::Result<PageDto> {
    let fmt = format(file);
    let Some(sides) = parse_vcs_marker_sides(content, fmt) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no VCS merge conflict markers to resolve",
        ));
    };
    // Scenario: malformed imported Org — a side that does not round-trip
    // would be rewritten lossily.
    if fmt == Format::Org
        && (!tine_core::org::org_editable(&sides.mine)
            || !tine_core::org::org_editable(&sides.theirs))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "an org side of this merge does not round-trip; not resolving",
        ));
    }
    let mine = parse(&sides.mine, fmt);
    let theirs = parse(&sides.theirs, fmt);
    let base = sides.base.as_deref().map(|text| parse(text, fmt));
    let artifact = sides.suggested.as_deref().map(|text| parse(text, fmt));
    let roots = sync_diff::merge_blocks3(
        base.as_ref().map(|doc| doc.roots.as_slice()),
        &mine.roots,
        &theirs.roots,
        artifact.as_ref().map(|doc| doc.roots.as_slice()),
        decisions,
    )
    .map_err(merge_refused)?;
    let pre_block = choose_pre(pre_choice, fmt, &mine, &theirs)?;
    Ok(dto(store, page, Document { pre_block, roots }))
}
