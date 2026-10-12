//! The derived conflict queue: duplicate journal days, sync copies and
//! VCS-marker pages as [`ConflictObject`]s, derived from disk once per open
//! graph and refreshed per published change. Split from `conflicts.rs`.

use std::io;

use tine_core::concord_queue::{
    decidable_row_count, vcs_conflict_markers, ConflictInventory, ConflictObject, ConflictSide,
    ConflictSource, MarkerConflictDiff, SideRole, VcsMarkerConflict,
};
use tine_core::model::{sync_conflict_base, PageKind, SyncConflict};
use tine_store::{Area, FileId, Store};

use super::{
    area_listing, duplicate_journal_diff, format, graph_text_title, id, sort_copies, store_failed,
    sync_conflict_diff, sync_copies, sync_copy_entry, sync_copy_winner, vcs_marker_conflict_diff,
    Unreadable,
};

/// Queue objects for every duplicate journal day: the keeper is Mine, each
/// stray a Theirs side, and the row count is the keeper against the FIRST
/// stray (a day with three files resolves pairwise; the queue re-derives with
/// one file fewer after each fold). Id `journal:<keeper path>` is stable for
/// the same disk state. Cost O(journal listing + duplicate file bytes).
fn journal_objects(store: &Store, bad: &mut Unreadable) -> io::Result<Vec<ConflictObject>> {
    let mut out = Vec::new();
    bad.journal_days.clear();
    for day in crate::journals::journal_conflicts(store)? {
        for file in &day.files {
            if let Some(reason) = &file.preview_error {
                bad.journal_days.insert(file.path.clone(), reason.clone());
            }
        }
        let [keeper, strays @ ..] = day.files.as_slice() else {
            continue;
        };
        let Some(first) = strays.first() else {
            continue;
        };
        let side = |role, file: &tine_core::model::JournalFile| ConflictSide {
            role,
            label: file.name.clone(),
            path: Some(file.path.clone()),
        };
        // A day whose pair cannot be diffed stays queued without row choices
        // (as a cross-format pair does), and the unreadable file is reported.
        let diff = match duplicate_journal_diff(store, &keeper.path, &first.path) {
            Ok(diff) => diff,
            Err(error) if store_failed(&error) => return Err(error),
            Err(error) => {
                if ![keeper, first].iter().any(|f| f.preview_error.is_some()) {
                    bad.journal_days
                        .insert(first.path.clone(), error.to_string());
                }
                None
            }
        };
        out.push(ConflictObject {
            id: format!("journal:{}", keeper.path),
            source: ConflictSource::DuplicateJournal,
            page_name: day.title.clone(),
            page_path: keeper.path.clone(),
            kind: PageKind::Journal,
            sides: std::iter::once(side(SideRole::Mine, keeper))
                .chain(strays.iter().map(|stray| side(SideRole::Theirs, stray)))
                .collect(),
            block_conflicts: diff.as_ref().map(|d| decidable_row_count(&d.rows)),
            markers: Vec::new(),
        });
    }
    Ok(out)
}

/// The marker listing entry for one page or journal file, or `None` when it
/// is not a graph page, is a sync copy, is unreadable, or carries no column-0
/// VCS marker. Such files stay real, readable pages; the store refuses to save
/// them (R-VCS-MARKERS). Read failures are errors, never absence: the caller
/// reports the file as unreadable and keeps the rest. Cost: O(1)
/// for a page the store has observed without an anchor line
/// (`Store::may_carry_vcs_markers`, no read); otherwise one bounded read, O(file
/// bytes), where a byte prefilter skips the UTF-8 check and line scan for files
/// without an anchor marker.
fn marker_entry(store: &Store, file: &FileId) -> io::Result<Option<VcsMarkerConflict>> {
    let Some((kind, stem, name)) = graph_text_title(store, file) else {
        return Ok(None);
    };
    if sync_conflict_base(&stem).is_some() {
        return Ok(None);
    }
    // A page the store has observed with no anchor line cannot be marker-
    // bearing: no read. (Cached pages are readable, within the parse limit and
    // valid UTF-8, so the read below could not have failed for them either.)
    if !store.may_carry_vcs_markers(file) {
        return Ok(None);
    }
    let (bytes, _) = match store.read(file, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
        Ok(value) => value,
        Err(tine_store::StoreError::NotFound) => return Ok(None),
        Err(error) => return Err(crate::store_error(error)),
    };
    if !tine_core::concord_queue::has_vcs_anchor(&bytes) {
        return Ok(None);
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let markers = vcs_conflict_markers(text, format(file));
    Ok((!markers.is_empty()).then(|| VcsMarkerConflict {
        path: file.as_str().to_owned(),
        name,
        kind,
        markers: markers.iter().map(|m| m.to_string()).collect(),
    }))
}

/// Marker-bearing pages. A file that cannot be read is skipped and reported;
/// a failed area scan propagates. Unreadable listing entries are recorded by
/// [`sync_copies`], which walks the same listings. Cost: O(pages) store
/// lookups plus one read per page that is uncached or may carry an anchor line
/// (none in an unmarked graph; was O(graph bytes), GH #623).
fn list_vcs_marker_pages(
    store: &Store,
    bad: &mut Unreadable,
) -> io::Result<Vec<VcsMarkerConflict>> {
    let mut out = Vec::new();
    for area in [Area::Journals, Area::Pages] {
        for entry in area_listing(store, area)?.0 {
            let found = marker_entry(store, &entry.id);
            out.extend(bad.file(entry.id.as_str(), found)?.flatten());
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The queue object for one listed copy; `None` for a stray whose winner is
/// gone (a one-sided file, not a conflict; the overview discards it). Cost:
/// one 2-way diff, O(both file bytes + blocks).
fn copy_object(store: &Store, copy: &SyncConflict) -> io::Result<Option<ConflictObject>> {
    let Some(winner) = copy.base_path.clone() else {
        return Ok(None);
    };
    let diff = sync_conflict_diff(store, &winner, &copy.path, &[])?;
    Ok(Some(ConflictObject {
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
    }))
}

/// The queue object for one marker-bearing page. Cost O(file bytes + blocks).
fn marker_object(store: &Store, marked: &VcsMarkerConflict) -> io::Result<ConflictObject> {
    let parsed = vcs_marker_conflict_diff(store, &marked.path)?;
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
    Ok(ConflictObject {
        id: format!("markers:{}", marked.path),
        source: ConflictSource::VcsMarkers,
        page_name: marked.name.clone(),
        page_path: marked.path.clone(),
        kind: marked.kind,
        sides,
        block_conflicts: parsed.as_ref().map(|p| decidable_row_count(&p.diff.rows)),
        markers: marked.markers.clone(),
    })
}

/// The graph config file as a store change names it.
const CONFIG: &str = "logseq/config.edn";

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
pub fn conflict_inventory(store: &Store) -> io::Result<ConflictInventory> {
    Ok(derive(store)?.published())
}

/// The inventory with what the walk could not read, kept by source.
#[derive(Clone)]
struct Derived {
    inventory: ConflictInventory,
    bad: Unreadable,
}

impl Derived {
    fn published(&self) -> ConflictInventory {
        ConflictInventory {
            unreadable: self.bad.rows(),
            ..self.inventory.clone()
        }
    }
}

fn derive(store: &Store) -> io::Result<Derived> {
    let mut bad = Unreadable::default();
    let sync_conflicts = sync_copies(store, &mut bad)?;
    let vcs_markers = list_vcs_marker_pages(store, &mut bad)?;
    let mut queue = journal_objects(store, &mut bad)?;
    for copy in &sync_conflicts {
        queue.extend(bad.file(&copy.path, copy_object(store, copy))?.flatten());
    }
    for marked in &vcs_markers {
        queue.extend(bad.file(&marked.path, marker_object(store, marked))?);
    }
    sort_queue(&mut queue);
    Ok(Derived {
        inventory: ConflictInventory {
            sync_conflicts,
            vcs_markers,
            queue,
            unreadable: Vec::new(),
        },
        bad,
    })
}

/// The derived conflict queue of one open graph, held in memory only and
/// never persisted. [`ConflictQueue::inventory`] walks the whole graph once
/// (at graph open) and afterwards answers from memory;
/// [`ConflictQueue::refresh_files`] re-derives only the entries a published
/// change can have affected. Every answer equals what [`conflict_inventory`]
/// would compute from the same disk state; resolves never trust it.
#[derive(Default)]
pub struct ConflictQueue {
    derived: std::sync::Mutex<Option<Derived>>,
    problem: std::sync::Mutex<Option<tine_store::IoError>>,
}

impl ConflictQueue {
    /// The inventory: one full [`conflict_inventory`] walk the first time
    /// (O(graph text bytes), about 72 ms on the 1,075-file anonymized graph),
    /// then a clone of the in-memory answer (O(conflicts)). The walk holds the
    /// queue's lock, so no concurrent [`Self::refresh_files`] can be lost.
    pub fn inventory(&self, store: &Store) -> io::Result<ConflictInventory> {
        let mut derived = self.derived.lock().unwrap_or_else(|e| e.into_inner());
        let mut problem = self.problem.lock().unwrap_or_else(|e| e.into_inner());
        if derived.is_none() || problem.is_some() {
            match derive(store) {
                Ok(value) => {
                    *derived = Some(value);
                    *problem = None;
                }
                Err(error) => {
                    *problem = Some(tine_store::IoError::from(io::Error::new(
                        error.kind(),
                        error.to_string(),
                    )));
                    return Err(error);
                }
            }
        }
        Ok(derived.as_ref().unwrap().published())
    }

    /// Re-derive what one published store change can affect. An external
    /// change (watcher, scan) re-derives every changed file, so a marker or
    /// copy written while the graph is open enters the queue on its next
    /// event. An own change (a Tine save) re-derives only files the queue
    /// already names or that are sync copies, so an ordinary save costs no
    /// read here. Returns whether the queue changed.
    pub fn refresh_change(&self, store: &Store, change: &tine_store::Change) -> io::Result<bool> {
        let files: Vec<FileId> = {
            let derived = self.derived.lock().unwrap_or_else(|e| e.into_inner());
            let Some(Derived { inventory, bad }) = derived.as_ref() else {
                return Ok(false);
            };
            let named = |path: &str| {
                sync_copy_winner(path).is_some()
                    || bad.files.contains_key(path)
                    || bad.journal_days.contains_key(path)
                    || inventory.vcs_markers.iter().any(|m| m.path == path)
                    || inventory.sync_conflicts.iter().any(|c| {
                        c.path == path || sync_copy_winner(&c.path).as_deref() == Some(path)
                    })
                    || path == CONFIG
                    || inventory.queue.iter().any(|o| {
                        o.source == ConflictSource::DuplicateJournal
                            && o.sides.iter().any(|s| s.path.as_deref() == Some(path))
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
        if files.is_empty() {
            Ok(false)
        } else {
            self.refresh_files(store, &files)
        }
    }

    /// Re-derive the entries `files` can affect: each file's own copy or
    /// marker entry, and every copy whose winner is one of them. A changed file
    /// that cannot be read is reported in `unreadable` (and leaves it once
    /// readable); only a closed store or failed area scan errors. Returns
    /// whether the queue or listings changed. Before the first
    /// [`Self::inventory`] it does nothing and returns `false` (that walk will
    /// read the current disk). Idempotent: it re-reads the files, so applying
    /// the same change twice is harmless. Cost: one bounded read per changed
    /// page or journal file plus one diff per affected copy, O(changed bytes);
    /// a changed journal file adds one journal-directory listing to re-derive
    /// the duplicate days (O(J)); no page-directory walk.
    pub fn refresh_files(&self, store: &Store, files: &[FileId]) -> io::Result<bool> {
        let mut derived = self.derived.lock().unwrap_or_else(|e| e.into_inner());
        let Some(current) = derived.as_ref() else {
            return Ok(false);
        };
        let before = serde_json::to_string(&current.published()).ok();
        let Derived {
            mut inventory,
            mut bad,
        } = current.clone();
        let result = (|| -> io::Result<()> {
            let exists = |rel: &str| -> io::Result<bool> {
                let file = id(store, rel)?;
                match store.open_read(&file) {
                    Ok(_) => Ok(true),
                    Err(tine_store::StoreError::NotFound) => Ok(false),
                    Err(error) => Err(crate::store_error(error)),
                }
            };
            let mut copies: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            // A config change can move the journal date formats, which re-titles
            // the duplicate days and can re-pick their keeper.
            let mut journals = files.iter().any(|f| f.as_str() == CONFIG);
            for file in files {
                let Some((kind, ..)) = graph_text_title(store, file) else {
                    continue;
                };
                journals |= kind == PageKind::Journal;
                let path = file.as_str();
                if sync_copy_winner(path).is_some() {
                    copies.insert(path.to_owned());
                    continue;
                }
                inventory.vcs_markers.retain(|m| m.path != path);
                bad.files.remove(path);
                let found = marker_entry(store, file);
                inventory
                    .vcs_markers
                    .extend(bad.file(path, found)?.flatten());
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
                bad.files.remove(copy);
                let file = FileId::from(copy.clone());
                let present = exists(copy);
                if bad.file(copy, present)?.unwrap_or(false) {
                    let entry = sync_copy_entry(store, &file, exists);
                    inventory
                        .sync_conflicts
                        .extend(bad.file(copy, entry)?.flatten());
                }
            }
            inventory.vcs_markers.sort_by(|a, b| a.path.cmp(&b.path));
            sort_copies(&mut inventory.sync_conflicts);
            let touched = |object: &ConflictObject| match object.source {
                ConflictSource::DuplicateJournal => journals,
                ConflictSource::SyncCopy => object
                    .sides
                    .iter()
                    .any(|s| s.path.as_ref().is_some_and(|p| copies.contains(p))),
                _ => files.iter().any(|f| f.as_str() == object.page_path),
            };
            inventory.queue.retain(|object| !touched(object));
            for copy in inventory
                .sync_conflicts
                .iter()
                .filter(|c| copies.contains(&c.path))
            {
                let object = copy_object(store, copy);
                inventory
                    .queue
                    .extend(bad.file(&copy.path, object)?.flatten());
            }
            for marked in inventory
                .vcs_markers
                .iter()
                .filter(|m| files.iter().any(|f| f.as_str() == m.path))
            {
                let object = marker_object(store, marked);
                inventory.queue.extend(bad.file(&marked.path, object)?);
            }
            if journals {
                bad.journals_listing = area_listing(store, Area::Journals)?.1;
                inventory.queue.extend(journal_objects(store, &mut bad)?);
            }
            sort_queue(&mut inventory.queue);
            Ok(())
        })();
        let mut problem = self.problem.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(()) => {
                let next = Derived { inventory, bad };
                let changed = serde_json::to_string(&next.published()).ok() != before;
                *derived = Some(next);
                *problem = None;
                Ok(changed)
            }
            Err(error) => {
                *problem = Some(io::Error::new(error.kind(), error.to_string()).into());
                Err(error)
            }
        }
    }
}
