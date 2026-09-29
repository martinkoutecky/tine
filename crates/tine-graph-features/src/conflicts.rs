//! Concord (og family 8): sync-copy and VCS-marker discovery, the derived
//! conflict queue, diffs, guarded merges, and recoverable trash. The conflict
//! copy is never treated as a graph page or entered into the cache. Nothing
//! here is persisted: the queue is recomputed from disk on every request.

use std::collections::HashMap;
use std::io;

use tine_core::concord_queue::{
    decidable_row_count, parse_vcs_marker_sides, vcs_conflict_markers, ConflictInventory,
    ConflictObject, ConflictSide, ConflictSource, MarkerConflictDiff, SideRole, VcsMarkerConflict,
};
use tine_core::date::JournalFormat;
use tine_core::doc::{self, Document};
use tine_core::model::{
    decode_page_name, sync_conflict_base, Format, PageDto, PageKind, SyncConflict,
};
use tine_core::projection::{assign_doc_runtime_ids, block_to_dto};
use tine_core::sync_diff::{self, SyncConflictDiff};
use tine_store::{Area, FileId, FileRev, PageId, SaveBase, Store};

pub(crate) fn invalid_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid file path")
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
    if id.as_str().ends_with(".org") {
        Format::Org
    } else {
        Format::Md
    }
}

pub(crate) fn parse(raw: &str, fmt: Format) -> Document {
    if fmt == Format::Org {
        tine_core::org::parse_org(raw)
    } else {
        doc::parse(raw)
    }
}

fn preview(store: &Store, file: &FileId) -> String {
    read_text(store, file)
        .ok()
        .and_then(|(content, _)| {
            content
                .lines()
                .map(|line| {
                    line.trim_start_matches(|ch| matches!(ch, '*' | '-' | ' ' | '\t'))
                        .trim()
                        .to_owned()
                })
                .find(|line| !line.is_empty())
        })
        .map(|line| line.chars().take(80).collect())
        .unwrap_or_default()
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
    winner_exists: impl FnOnce(&str) -> bool,
) -> Option<SyncConflict> {
    let (kind, stem, base_name) = graph_text_title(store, file)?;
    let base_stem = sync_conflict_base(&stem)?;
    let winner = sync_copy_winner(file.as_str())?;
    let tag = stem[base_stem.len()..]
        .trim_matches(|ch: char| matches!(ch, '.' | ' ' | '(' | ')'))
        .to_owned();
    Some(SyncConflict {
        path: file.as_str().to_owned(),
        base_name,
        base_path: winner_exists(&winner).then_some(winner),
        kind,
        tag,
        preview: preview(store, file),
    })
}

fn sort_copies(out: &mut [SyncConflict]) {
    out.sort_by(|a, b| {
        a.base_name
            .cmp(&b.base_name)
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// List Syncthing, Dropbox and Seafile copies in pages and journals.
/// Unreadable scan entries are skipped and unreadable previews are empty.
/// Cost O(P + J + conflict-copy bytes), including winner presence checks.
pub fn list_sync_conflicts(store: &Store) -> Vec<SyncConflict> {
    let mut out = Vec::new();
    for area in [Area::Journals, Area::Pages] {
        let Ok(listing) = store.scan_area(area, None) else {
            continue;
        };
        let present: std::collections::HashSet<_> = listing
            .files
            .iter()
            .map(|entry| entry.id.as_str().to_owned())
            .collect();
        for entry in &listing.files {
            out.extend(sync_copy_entry(store, &entry.id, |winner| {
                present.contains(winner)
            }));
        }
    }
    sort_copies(&mut out);
    out
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

/// A pre-block property line as (lowercase key, value): Markdown `key:: value`,
/// Org `#+KEY: value`.
fn pre_property(line: &str, fmt: Format) -> Option<(String, &str)> {
    if fmt == Format::Md {
        return doc::parse_property_line(line).map(|(k, v)| (k.to_ascii_lowercase(), v));
    }
    let (key, value) = line.trim_start().strip_prefix("#+")?.split_once(':')?;
    (!key.is_empty() && !key.contains(char::is_whitespace))
        .then(|| (key.to_ascii_lowercase(), value.trim()))
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
    let mine_lines: Vec<&str> = mine.split_inclusive('\n').collect();
    let body = |l: &str| l.trim_end_matches(['\r', '\n']).to_owned();
    let mine_props: HashMap<String, (usize, &str)> = mine_lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let l = l.trim_end_matches(['\r', '\n']);
            pre_property(l, fmt).map(|(k, v)| (k, (i, v)))
        })
        .collect();
    let mine_text: std::collections::HashSet<String> = mine_lines
        .iter()
        .map(|l| body(l).trim().to_owned())
        .collect();
    let mut extra_props = Vec::new();
    let mut extra_text = Vec::new();
    let mut members: HashMap<usize, Vec<String>> = HashMap::new();
    let mut clashes = Vec::new();
    for line in theirs.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || mine_text.contains(trimmed) {
            continue;
        }
        match pre_property(line, fmt) {
            Some((key, value)) => match mine_props.get(&key) {
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
                Some(_) => clashes.push(key),
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
        .take_while(|l| pre_property(l.trim_end_matches(['\r', '\n']), fmt).is_some())
        .count();
    let mut out = String::with_capacity(mine.len() + theirs.len());
    let push_line = |out: &mut String, line: &str| {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(line);
    };
    for (i, line) in mine_lines.iter().enumerate() {
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
    let page = store.as_page(&win).ok_or_else(invalid_path)?;
    crate::retry_on_conflict("conflict files changed repeatedly during merge", || {
        let (mine, win_rev) = read_text(store, &win)?;
        let (theirs, conf_rev) = read_text(store, &conf)?;
        if String::from(win_rev.clone()) != base_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "winner changed on disk",
            ));
        }
        if String::from(conf_rev.clone()) != conflict_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "conflict copy changed on disk",
            ));
        }
        let fmt = format(&win);
        if fmt == Format::Org
            && (!tine_core::org::org_editable(&mine) || !tine_core::org::org_editable(&theirs))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "an org file in this pair does not round-trip; not merging",
            ));
        }
        let mine_doc = parse(&mine, fmt);
        let their_doc = parse(&theirs, format(&conf));
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
        let merged = dto(store, &page, Document { pre_block, roots });
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[
                tine_store::EditKind::ReplacePage,
                tine_store::EditKind::DeletePage,
            ],
            &page,
            SaveBase::Existing(win_rev),
            &merged,
        );
        tx.trash(&conf, conf_rev);
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

/// Recoverably trash only a sync copy, with four revision-guard retries. Cost
/// O(copy bytes) per attempt.
pub fn trash_sync_conflict(store: &Store, conflict: &str) -> io::Result<()> {
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
        crate::trash_current(
            store,
            &conf,
            Some(tine_store::PARSE_INPUT_MAX_BYTES),
            "no such conflict file",
        )
    })
}

/// The marker listing entry for one page or journal file, or `None` when it
/// is not a graph page, is a sync copy, is unreadable, or carries no column-0
/// VCS marker. Such files stay real, readable pages; the store refuses to save
/// them (R-VCS-MARKERS). An unreadable file is not an error here: an inventory
/// read never refuses. Cost: one bounded read, O(file bytes); a byte prefilter
/// skips the UTF-8 check and line scan for files without an anchor marker.
fn marker_entry(store: &Store, file: &FileId) -> Option<VcsMarkerConflict> {
    let (kind, stem, name) = graph_text_title(store, file)?;
    if sync_conflict_base(&stem).is_some() {
        return None;
    }
    let (bytes, _) = store
        .read(file, Some(tine_store::PARSE_INPUT_MAX_BYTES))
        .ok()?;
    if !has_anchor(&bytes) {
        return None;
    }
    let markers = vcs_conflict_markers(std::str::from_utf8(&bytes).ok()?);
    (!markers.is_empty()).then(|| VcsMarkerConflict {
        path: file.as_str().to_owned(),
        name,
        kind,
        markers: markers.iter().map(|m| m.to_string()).collect(),
    })
}

/// Page and journal files carrying unresolved VCS markers. Sync-tool copies
/// are listed by [`list_sync_conflicts`] instead. Cost: one bounded read of
/// every page and journal file, O(graph text bytes).
fn list_vcs_marker_pages(store: &Store) -> Vec<VcsMarkerConflict> {
    let mut out = Vec::new();
    for area in [Area::Journals, Area::Pages] {
        let Ok(listing) = store.scan_area(area, None) else {
            continue;
        };
        out.extend(
            listing
                .files
                .iter()
                .filter_map(|entry| marker_entry(store, &entry.id)),
        );
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Whether `bytes` contain a column-0 anchor marker line (`<<<<<<< ` or
/// `>>>>>>> `); the exact rules live in `vcs_conflict_markers`.
fn has_anchor(bytes: &[u8]) -> bool {
    bytes
        .split(|&b| b == b'\n')
        .any(|line| line.starts_with(b"<<<<<<< ") || line.starts_with(b">>>>>>> "))
}

/// The queue object for one listed copy; `None` for a stray whose winner is
/// gone (a one-sided file, not a conflict; the overview discards it). Cost:
/// one 2-way diff, O(both file bytes + blocks).
fn copy_object(store: &Store, copy: &SyncConflict) -> Option<ConflictObject> {
    let winner = copy.base_path.clone()?;
    let diff = sync_conflict_diff(store, &winner, &copy.path, &[])
        .ok()
        .flatten();
    Some(ConflictObject {
        id: format!("copy:{}", copy.path),
        source: ConflictSource::SyncCopy,
        page_name: copy.base_name.clone(),
        page_path: winner.clone(),
        kind: copy.kind,
        sides: vec![
            ConflictSide {
                role: SideRole::Mine,
                label: "This device".to_string(),
                path: Some(winner),
            },
            ConflictSide {
                role: SideRole::Theirs,
                label: if copy.tag.is_empty() {
                    "Conflict copy".to_string()
                } else {
                    copy.tag.clone()
                },
                path: Some(copy.path.clone()),
            },
        ],
        block_conflicts: diff.as_ref().map(|d| decidable_row_count(&d.rows)),
        markers: Vec::new(),
    })
}

/// The queue object for one marker-bearing page. Cost O(file bytes + blocks).
fn marker_object(store: &Store, marked: &VcsMarkerConflict) -> ConflictObject {
    let parsed = vcs_marker_conflict_diff(store, &marked.path).ok().flatten();
    let label = |pick: fn(&MarkerConflictDiff) -> &str, fallback: &str| {
        parsed
            .as_ref()
            .map(pick)
            .filter(|l| !l.is_empty())
            .unwrap_or(fallback)
            .to_string()
    };
    let mut sides = vec![
        ConflictSide {
            role: SideRole::Mine,
            label: label(|p| p.mine_label.as_str(), "Local side"),
            path: None,
        },
        ConflictSide {
            role: SideRole::Theirs,
            label: label(|p| p.theirs_label.as_str(), "Merged-in side"),
            path: None,
        },
    ];
    if parsed.as_ref().is_some_and(|p| p.diff.three_way) {
        sides.push(ConflictSide {
            role: SideRole::Base,
            label: "Common ancestor".to_string(),
            path: None,
        });
    }
    ConflictObject {
        id: format!("markers:{}", marked.path),
        source: ConflictSource::VcsMarkers,
        page_name: marked.name.clone(),
        page_path: marked.path.clone(),
        kind: marked.kind,
        sides,
        block_conflicts: parsed.as_ref().map(|p| decidable_row_count(&p.diff.rows)),
        markers: marked.markers.clone(),
    }
}

fn sort_queue(queue: &mut [ConflictObject]) {
    queue.sort_by(|a, b| a.page_name.cmp(&b.page_name).then_with(|| a.id.cmp(&b.id)));
}

/// The Concord conflict queue: ONE derived inventory of everything on disk
/// that needs the user's judgement — sync-tool copies paired with their
/// winner, and marker-bearing pages. Never persisted and never an authority:
/// the same disk state recomputes the same objects with the same ids, and
/// every resolve re-checks the files it writes. A copy whose winner is gone is
/// a stray, not a two-sided conflict, and stays out (the overview discards it).
/// Cost: the two whole-graph listings (one bounded read of every page and
/// journal file) plus one diff per queued item (conflicts are few).
pub fn conflict_inventory(store: &Store) -> ConflictInventory {
    let sync_conflicts = list_sync_conflicts(store);
    let vcs_markers = list_vcs_marker_pages(store);
    let mut queue: Vec<_> = sync_conflicts
        .iter()
        .filter_map(|copy| copy_object(store, copy))
        .chain(
            vcs_markers
                .iter()
                .map(|marked| marker_object(store, marked)),
        )
        .collect();
    sort_queue(&mut queue);
    ConflictInventory {
        sync_conflicts,
        vcs_markers,
        queue,
    }
}

/// The derived conflict queue of one open graph, held in memory only and
/// never persisted. [`ConflictQueue::inventory`] walks the whole graph once
/// (at graph open) and afterwards answers from memory;
/// [`ConflictQueue::refresh_files`] re-derives only the entries a published
/// change can have affected. Every answer equals what [`conflict_inventory`]
/// would compute from the same disk state; resolves never trust it.
#[derive(Default)]
pub struct ConflictQueue {
    derived: std::sync::Mutex<Option<ConflictInventory>>,
}

impl ConflictQueue {
    /// The inventory: one full [`conflict_inventory`] walk the first time
    /// (O(graph text bytes), about 72 ms on the 1,075-file anonymized graph),
    /// then a clone of the in-memory answer (O(conflicts)). The walk holds the
    /// queue's lock, so no concurrent [`Self::refresh_files`] can be lost.
    pub fn inventory(&self, store: &Store) -> ConflictInventory {
        let mut derived = self.derived.lock().unwrap_or_else(|e| e.into_inner());
        derived
            .get_or_insert_with(|| conflict_inventory(store))
            .clone()
    }

    /// Re-derive what one published store change can affect. An external
    /// change (watcher, scan) re-derives every changed file, so a marker or
    /// copy written while the graph is open enters the queue on its next
    /// event. An own change (a Tine save) re-derives only files the queue
    /// already names or that are sync copies, so an ordinary save costs no
    /// read here. Returns whether the queue changed.
    pub fn refresh_change(&self, store: &Store, change: &tine_store::Change) -> bool {
        let files: Vec<FileId> = {
            let derived = self.derived.lock().unwrap_or_else(|e| e.into_inner());
            let Some(inventory) = derived.as_ref() else {
                return false;
            };
            let named = |path: &str| {
                sync_copy_winner(path).is_some()
                    || inventory.vcs_markers.iter().any(|m| m.path == path)
                    || inventory.sync_conflicts.iter().any(|c| {
                        c.path == path || sync_copy_winner(&c.path).as_deref() == Some(path)
                    })
            };
            change
                .files
                .iter()
                .filter(|(file, _, _)| {
                    change.origin == tine_store::Origin::External || named(file.as_str())
                })
                .map(|(file, _, _)| file.clone())
                .collect()
        };
        !files.is_empty() && self.refresh_files(store, &files)
    }

    /// Re-derive the entries `files` can affect: each file's own copy or
    /// marker entry, and every copy whose winner is one of them. Returns
    /// whether the queue or listings changed. Before the first
    /// [`Self::inventory`] it does nothing and returns `false` (that walk will
    /// read the current disk). Idempotent: it re-reads the files, so applying
    /// the same change twice is harmless. Cost: one bounded read per changed
    /// page or journal file plus one diff per affected copy, O(changed bytes);
    /// no directory walk.
    pub fn refresh_files(&self, store: &Store, files: &[FileId]) -> bool {
        let mut derived = self.derived.lock().unwrap_or_else(|e| e.into_inner());
        let Some(inventory) = derived.as_mut() else {
            return false;
        };
        let before = serde_json::to_string(&*inventory).ok();
        let exists = |rel: &str| id(store, rel).is_ok_and(|file| store.open_read(&file).is_ok());
        let mut copies: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for file in files {
            if graph_text_title(store, file).is_none() {
                continue;
            }
            let path = file.as_str();
            if sync_copy_winner(path).is_some() {
                copies.insert(path.to_owned());
                continue;
            }
            inventory.vcs_markers.retain(|m| m.path != path);
            inventory.vcs_markers.extend(marker_entry(store, file));
            copies.extend(
                inventory
                    .sync_conflicts
                    .iter()
                    .filter(|c| sync_copy_winner(&c.path).as_deref() == Some(path))
                    .map(|c| c.path.clone()),
            );
        }
        for copy in &copies {
            inventory.sync_conflicts.retain(|c| &c.path != copy);
            let file = FileId::from(copy.clone());
            if exists(copy) {
                inventory
                    .sync_conflicts
                    .extend(sync_copy_entry(store, &file, exists));
            }
        }
        inventory.vcs_markers.sort_by(|a, b| a.path.cmp(&b.path));
        sort_copies(&mut inventory.sync_conflicts);
        let touched = |object: &ConflictObject| match object.source {
            ConflictSource::SyncCopy => object
                .sides
                .iter()
                .any(|s| s.path.as_ref().is_some_and(|p| copies.contains(p))),
            _ => files.iter().any(|f| f.as_str() == object.page_path),
        };
        inventory.queue.retain(|object| !touched(object));
        let fresh: Vec<_> = inventory
            .sync_conflicts
            .iter()
            .filter(|c| copies.contains(&c.path))
            .filter_map(|c| copy_object(store, c))
            .chain(
                inventory
                    .vcs_markers
                    .iter()
                    .filter(|m| files.iter().any(|f| f.as_str() == m.path))
                    .map(|m| marker_object(store, m)),
            )
            .collect();
        inventory.queue.extend(fresh);
        sort_queue(&mut inventory.queue);
        serde_json::to_string(&*inventory).ok() != before
    }
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
    let Some(sides) = parse_vcs_marker_sides(&content) else {
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
    rel: &str,
    decisions: &HashMap<String, String>,
    base_rev: &str,
    pre_choice: &str,
) -> io::Result<()> {
    let file = id(store, rel)?;
    let page = store.as_page(&file).ok_or_else(invalid_path)?;
    crate::retry_on_conflict("marker file changed repeatedly during resolve", || {
        let (content, rev) = read_text(store, &file)?;
        // Scenario: the VCS or an external editor changed the file after the
        // review was computed; the user must review the new version.
        if String::from(rev.clone()) != base_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "file changed on disk",
            ));
        }
        let Some(sides) = parse_vcs_marker_sides(&content) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no VCS merge conflict markers to resolve",
            ));
        };
        let fmt = format(&file);
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
        let merged = dto(store, &page, Document { pre_block, roots });
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[tine_store::EditKind::ReplacePage],
            &page,
            SaveBase::ResolvingMarkers(rev),
            &merged,
        );
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}
