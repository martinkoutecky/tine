//! PDF sidecars, annotation pages, view state, and cropped images. Guarded
//! overwrites retry a concurrent external revision change at most four times.
//! Crop rollback checks the current sidecar and crop in one guarded transaction.
//! Missing or invalid sidecars/crops, I/O failures, and repeated conflicts are
//! returned to the caller.

use std::collections::{HashMap, HashSet};
use std::io;
use tine_core::model::{Format, PageDto, PageKind};
use tine_core::pdf::{self, Highlight, PdfState};
use tine_store::{
    Area, Content, FileId, FileRev, Input, PageHost, PageId, SaveBase, Store, StoreError,
};

use crate::store_error;

fn asset(store: &Store, rel: &str) -> io::Result<FileId> {
    store.file_id(Area::Assets, rel).map_err(store_error)
}

fn optional(store: &Store, id: &FileId) -> io::Result<Option<(String, FileRev)>> {
    match crate::parsed_text::read(store, id) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn merge_highlight(loaded: &Highlight, local: &Highlight, disk: &Highlight) -> Highlight {
    let local_geometry = local.page != loaded.page || local.position != loaded.position;
    Highlight {
        id: local.id.clone(),
        page: if local_geometry {
            local.page
        } else {
            disk.page
        },
        position: if local_geometry {
            &local.position
        } else {
            &disk.position
        }
        .clone(),
        color: if local.color != loaded.color {
            &local.color
        } else {
            &disk.color
        }
        .clone(),
        text: if local.text != loaded.text {
            &local.text
        } else {
            &disk.text
        }
        .clone(),
        image: if local.image != loaded.image {
            local.image
        } else {
            disk.image
        },
    }
}

#[cfg(test)]
#[path = "pdf_tests.rs"]
mod tests;

fn valid_edn(raw: &str) -> io::Result<()> {
    if raw.trim().is_empty()
        || matches!(
            tine_core::edn::parse_strict(raw),
            Some(tine_core::edn::Edn::Map(_))
        )
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "highlight sidecar is malformed; refusing to replace it",
        ))
    }
}

fn legacy_active(store: &Store, pdf_name: &str) -> bool {
    let legacy = pdf::legacy_asset_key(pdf_name);
    if legacy == pdf::asset_key(pdf_name) {
        return false;
    }
    let Ok(listing) = store.scan_area(Area::Assets, None) else {
        return true;
    };
    !listing.files.iter().any(|entry| {
        !entry.rel.contains('/')
            && (entry.rel.ends_with(".pdf") || entry.rel.ends_with(".PDF"))
            && pdf::asset_key(&entry.rel) == legacy
    })
}

fn sidecar(
    store: &Store,
    pdf_name: &str,
    allow_legacy: bool,
) -> io::Result<(FileId, Option<(String, FileRev)>)> {
    let key = pdf::asset_key(pdf_name);
    let primary = asset(store, &format!("{key}.edn"))?;
    let present = optional(store, &primary)?;
    if present.is_some() || !allow_legacy || !legacy_active(store, pdf_name) {
        return Ok((primary, present));
    }
    let legacy = pdf::legacy_asset_key(pdf_name);
    let old = asset(store, &format!("{legacy}.edn"))?;
    if let Some(value) = optional(store, &old)? {
        Ok((old, Some(value)))
    } else {
        Ok((primary, None))
    }
}

fn page_id(store: &Store, name: &str) -> io::Result<(PageId, Option<(String, FileRev)>)> {
    let md = store
        .file_id(Area::Pages, &format!("{name}.md"))
        .map_err(store_error)?;
    let org = store
        .file_id(Area::Pages, &format!("{name}.org"))
        .map_err(store_error)?;
    let a = optional(store, &md)?;
    let b = optional(store, &org)?;
    if a.is_some() && b.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("page has both .md and .org files: {name}"),
        ));
    }
    if let Some(value) = a {
        return Ok((store.as_page(&md).expect("md page"), Some(value)));
    }
    if let Some(value) = b {
        return Ok((store.as_page(&org).expect("org page"), Some(value)));
    }
    store
        .refresh(tine_store::Depth::Stamps)
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
    // A page found by name under another file name (an encoded one: `%`,
    // `#`, ...) is written over its current text, not created.
    match store
        .whole_graph()
        .map_err(|_| io::Error::other("graph unavailable"))?
        .resolve(name, false)
    {
        tine_store::Resolved::Existing { id, .. } => {
            let existing = optional(store, &id.file())?;
            Ok((id, existing))
        }
        tine_store::Resolved::Absent { id } => Ok((id, None)),
        tine_store::Resolved::Alias { .. } => Ok((store.as_page(&md).expect("md page"), None)),
    }
}

fn format(id: &PageId) -> Format {
    Format::from_path(id.as_str().as_ref())
}

fn parse_doc(raw: &str, fmt: Format) -> tine_core::doc::Document {
    if fmt == Format::Org {
        tine_core::org::parse_org(raw)
    } else {
        tine_core::doc::parse(raw)
    }
}

fn dto(id: &PageId, name: &str, doc: &tine_core::doc::Document) -> PageDto {
    let mut doc = doc.clone();
    tine_core::projection::assign_doc_runtime_ids(&mut doc.roots, id.as_str());
    PageDto {
        name: name.to_owned(),
        kind: PageKind::Page,
        title: name.to_owned(),
        pre_block: doc.pre_block.clone(),
        blocks: doc
            .roots
            .iter()
            .map(tine_core::projection::block_to_dto)
            .collect(),
        rev: None,
        format: format(id),
        read_only: false,

        guide: false,
    }
}

/// Read highlights from the OG-key sidecar or the legacy-key fallback. Bad or
/// absent files return an empty set as in v0.6.5. Cost O(sidecar bytes).
pub fn read_highlights(store: &Store, pdf_name: &str) -> Vec<Highlight> {
    let key = pdf::asset_key(pdf_name);
    let legacy = pdf::legacy_asset_key(pdf_name);
    let primary = asset(store, &format!("{key}.edn"))
        .ok()
        .and_then(|id| optional(store, &id).ok().flatten());
    let fallback = if primary.is_none() && legacy != key {
        asset(store, &format!("{legacy}.edn"))
            .ok()
            .and_then(|id| optional(store, &id).ok().flatten())
    } else {
        None
    };
    primary
        .or(fallback)
        .map(|(raw, _)| pdf::parse_highlights(&raw))
        .unwrap_or_default()
}

/// Read the current PDF sidecar, including an active legacy sidecar. Missing
/// files return an empty set. Unreadable or malformed nonblank top-level EDN
/// rejects; malformed highlight entries within a valid map are skipped.
/// Cost O(asset entries plus sidecar bytes). No write is performed.
pub fn read_highlights_checked(store: &Store, pdf_name: &str) -> io::Result<Vec<Highlight>> {
    let (_, current) = sidecar(store, pdf_name, true)?;
    let Some((raw, _)) = current else {
        return Ok(Vec::new());
    };
    valid_edn(&raw)?;
    Ok(pdf::parse_highlights(&raw))
}

/// Read persisted PDF highlights and view state without creating, rewriting or
/// moving any graph files. Prefer the OG-key sidecar; only when absent, consult
/// legacy unless another PDF owns its key. An unavailable asset listing permits
/// legacy lookup. Missing files return empty state; malformed nonblank EDN or
/// sidecar I/O rejects. The label does not affect this read.
/// Annotation pages are created only by the guarded highlight writer.
/// Cost O(asset entries + sidecar bytes); no graph refresh is needed.
pub fn open_pdf(store: &Store, pdf_name: &str, _label: &str) -> io::Result<PdfState> {
    let (_, current) = sidecar(store, pdf_name, true)?;
    let raw = current.map(|(raw, _)| raw).unwrap_or_default();
    valid_edn(&raw)?;
    Ok(pdf::parse_pdf_state(&raw))
}

fn area_image_target(
    store: &Store,
    pdf_name: &str,
    page: i64,
    id: &str,
    stamp: i64,
) -> io::Result<(FileId, String)> {
    let name = format!("{page}_{id}_{stamp}.png");
    if name.contains('/') || name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad asset name",
        ));
    }
    let key = pdf::asset_key(pdf_name);
    let rel = format!("{key}/{name}");
    let file = asset(store, if key.is_empty() { &name } else { &rel })?;
    Ok((file, rel))
}

/// Write a crop under its stable `key/page_id_stamp.png` link. A repeat save
/// replaces that file in place; concurrent external writes are retried four
/// times. Each attempt reads the current crop; cost O(existing + input bytes).
pub fn write_pdf_area_image(
    store: &Store,
    pdf_name: &str,
    page: i64,
    id: &str,
    stamp: i64,
    bytes: &[u8],
) -> io::Result<String> {
    let (file, rel) = area_image_target(store, pdf_name, page, id, stamp)?;
    crate::retry_on_conflict("PDF area image changed repeatedly during save", || {
        let baseline = match store.read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
            Ok((_, rev)) => Some(rev),
            Err(StoreError::NotFound) => None,
            Err(error) => return Err(store_error(error)),
        };
        let mut tx = store.transaction(None);
        match baseline {
            Some(rev) => {
                tx.replace(&file, rev, bytes.to_vec());
            }
            None => {
                tx.create(&file, Content::Bytes(bytes.to_vec()));
            }
        }
        Ok(crate::commit_retry(tx.commit())?.then_some(rel.clone()))
    })
}

const LEGACY_SIDECAR_LEFT: &str =
    "tine pdf: highlights saved; the old-key highlight sidecar could not be retired and was left in place";
const LEGACY_PAGE_LEFT: &str =
    "tine pdf: highlights saved; the old-key annotation page could not be retired and was left in place";
const DELETED_CROP_LEFT: &str =
    "tine pdf: highlights saved; a deleted area highlight's image could not be removed and was left in place";

/// Post-commit retirement is best effort: the save already succeeded, so a
/// failed cleanup does not fail it. It does not vanish either (I-9): the fixed
/// line (I-5: no names) reaches stderr and the host's `--debug` log.
fn report_cleanup(outcome: tine_store::TxOutcome, line: &'static str) {
    if !matches!(crate::commit_retry(outcome), Ok(true)) {
        tine_core::diag_line::diagnostic_line(line);
    }
}

/// Trash a crop only when the current primary sidecar has no reference to its
/// ID and stamp. A read-only sidecar revision check runs in the same
/// transaction as the crop trash. Missing/malformed sidecars and I/O
/// failures refuse; conflicts retry four full reads. Cost O(sidecar + crop
/// bytes) per attempt.
pub fn rollback_pdf_area_image(
    store: &Store,
    pdf_name: &str,
    page: i64,
    id: &str,
    stamp: i64,
) -> io::Result<()> {
    let (file, _) = area_image_target(store, pdf_name, page, id, stamp)?;
    rollback_pdf_area_file(store, pdf_name, &file, id, stamp)
}

fn rollback_pdf_area_file(
    store: &Store,
    pdf_name: &str,
    file: &FileId,
    id: &str,
    stamp: i64,
) -> io::Result<()> {
    crate::retry_on_conflict("PDF area image changed repeatedly during rollback", || {
        let sidecar_id = asset(store, &format!("{}.edn", pdf::asset_key(pdf_name)))?;
        let (raw, sidecar_rev) = optional(store, &sidecar_id)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "PDF sidecar missing; refusing crop rollback",
            )
        })?;
        valid_edn(&raw)?;
        let parsed = tine_core::edn::parse_strict(&raw).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "PDF sidecar missing highlights")
        })?;
        let entries = parsed
            .get("highlights")
            .and_then(tine_core::edn::Edn::as_vec)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "PDF sidecar missing highlights")
            })?;
        let highlights = pdf::parse_highlights(&raw);
        if highlights.len() != entries.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PDF sidecar has unrecognized highlight entries",
            ));
        }
        if highlights
            .iter()
            .any(|highlight| highlight.id == id && highlight.image == Some(stamp))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PDF sidecar still references area image; refusing crop rollback",
            ));
        }
        let crop_rev = store
            .read(file, Some(tine_store::PARSE_INPUT_MAX_BYTES))
            .map_err(store_error)?
            .1;
        let mut tx = store.transaction(None);
        tx.expect(&sidecar_id, sidecar_rev);
        tx.trash(file, crop_rev, tine_store::TrashIf::Any);
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

/// The caller's highlights merged against the sidecar's current text.
struct Merged {
    /// The sidecar's next highlights, and the caller's next baseline.
    highlights: Vec<Highlight>,
    /// The sidecar's highlights before this write.
    disk: Vec<Highlight>,
    /// The highlights this write knows were deleted: their blocks go.
    removed: HashSet<String>,
}

/// Merge `highlights` against the sidecar text `raw` (see
/// [`write_highlights`]); an edit against a concurrent deletion, or a
/// deletion against a concurrent edit, refuses. Writes nothing.
fn merge(
    raw: &str,
    highlights: &[Highlight],
    base: &HashMap<&str, &Highlight>,
) -> io::Result<Merged> {
    valid_edn(raw)?;
    let disk = pdf::parse_highlights(raw);
    let have: HashSet<&str> = highlights.iter().map(|h| h.id.as_str()).collect();
    let disk_by_id: HashMap<&str, &Highlight> = disk.iter().map(|h| (h.id.as_str(), h)).collect();
    if highlights.iter().any(|local| {
        base.get(local.id.as_str())
            .is_some_and(|loaded| *loaded != local && !disk_by_id.contains_key(local.id.as_str()))
    }) || base.iter().any(|(id, loaded)| {
        !have.contains(id)
            && disk_by_id
                .get(id)
                .is_some_and(|current| *current != *loaded)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "highlight edit conflicts with a concurrent deletion or edit; local changes remain unsaved",
        ));
    }
    let mut merged: Vec<Highlight> = highlights
        .iter()
        .filter_map(|local| {
            match (
                base.get(local.id.as_str()),
                disk_by_id.get(local.id.as_str()),
            ) {
                (Some(loaded), Some(current)) => Some(merge_highlight(loaded, local, current)),
                (Some(loaded), None) if *loaded == local => None,
                _ => Some(local.clone()),
            }
        })
        .collect();
    for item in &disk {
        if !have.contains(item.id.as_str()) && !base.contains_key(item.id.as_str()) {
            merged.push(item.clone());
        }
    }
    // Only highlights this write knows were deleted lose their page block;
    // an annotation whose sidecar entry is unreadable or not yet synced is
    // not ours to remove (L01 H1/H2).
    let merged_ids: HashSet<&str> = merged.iter().map(|item| item.id.as_str()).collect();
    let removed: HashSet<String> = disk
        .iter()
        .map(|item| item.id.as_str())
        .chain(base.keys().copied())
        .filter(|id| !merged_ids.contains(id))
        .map(str::to_owned)
        .collect();
    Ok(Merged {
        highlights: merged,
        disk,
        removed,
    })
}

type Read = Option<(String, FileRev)>;

/// The sidecar a write reads: the OG-key one, and only when it is absent,
/// the legacy-key one (`old`).
fn sidecars(store: &Store, primary: &FileId, legacy: Option<&FileId>) -> io::Result<(Read, Read)> {
    let current = optional(store, primary)?;
    let old = if current.is_none() {
        legacy.map(|id| optional(store, id)).transpose()?.flatten()
    } else {
        None
    };
    Ok((current, old))
}

/// The annotation page a write targets, and the text it is computed from.
struct Annotation {
    page: PageId,
    /// The page's current text, when it has a file.
    existing: Read,
    /// With no OG-key page: the legacy-key page whose text the new one takes.
    legacy: Option<(PageId, String, FileRev)>,
}

impl Annotation {
    fn prior(&self) -> Option<&str> {
        let legacy = self.legacy.as_ref().map(|(_, raw, _)| raw.as_str());
        self.existing
            .as_ref()
            .map(|(raw, _)| raw.as_str())
            .or(legacy)
    }
}

/// The annotation page `name`, or with `migrate` and no such page, a new
/// page under `name` carrying the legacy page `old_name`'s text. A
/// non-round-tripping Org page refuses. Reads only.
fn annotation(store: &Store, name: &str, old_name: &str, migrate: bool) -> io::Result<Annotation> {
    let (mut page, existing) = page_id(store, name)?;
    let legacy = match (&existing, migrate) {
        (None, true) => match page_id(store, old_name)? {
            (id, Some((raw, rev))) => Some((id, raw, rev)),
            (_, None) => None,
        },
        _ => None,
    };
    if let Some((id, ..)) = &legacy {
        let ext = if format(id) == Format::Org {
            "org"
        } else {
            "md"
        };
        let file = store
            .file_id(Area::Pages, &format!("{name}.{ext}"))
            .map_err(store_error)?;
        page = store.as_page(&file).expect("annotation page");
    }
    let annotation = Annotation {
        page,
        existing,
        legacy,
    };
    if format(&annotation.page) == Format::Org
        && annotation
            .prior()
            .is_some_and(|raw| !tine_core::org::org_editable(raw))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "org highlight page is read-only (does not round-trip)",
        ));
    }
    Ok(annotation)
}

/// The annotation page's next content for `merged`.
fn annotation_dto(
    annotation: &Annotation,
    name: &str,
    pdf_name: &str,
    label: &str,
    merged: &Merged,
) -> PageDto {
    let fmt = format(&annotation.page);
    let prior = annotation.prior().map(|raw| parse_doc(raw, fmt));
    let doc = pdf::merge_hls_page_for_format(
        prior.as_ref(),
        pdf_name,
        label,
        &merged.highlights,
        &merged.removed,
        fmt,
    );
    dto(&annotation.page, name, &doc)
}

/// The sidecar's steps in `tx`: the OG-key sidecar replaced (or created)
/// with `next`, and a legacy sidecar read replaced by its own bytes, a
/// revision guard.
fn sidecar_steps(
    tx: &mut tine_store::Transaction<'_>,
    primary: &FileId,
    current: Option<&(String, FileRev)>,
    next: &str,
    legacy: Option<(&FileId, &(String, FileRev))>,
) {
    match current {
        Some((_, rev)) => {
            tx.replace(primary, rev.clone(), next.as_bytes().to_vec());
        }
        None => {
            tx.create(primary, Content::Bytes(next.as_bytes().to_vec()));
        }
    }
    if let Some((id, (raw, rev))) = legacy {
        tx.replace(id, rev.clone(), raw.clone().into_bytes());
    }
}

/// With a host, the sidecar's own guarded transaction, before the
/// annotation page's host operation (DESIGN §3.4 pre). Assets only.
fn save_sidecar(
    store: &Store,
    primary: &FileId,
    current: Option<&(String, FileRev)>,
    next: &str,
    legacy: Option<(&FileId, &(String, FileRev))>,
) -> io::Result<bool> {
    let mut tx = store.transaction(None);
    sidecar_steps(&mut tx, primary, current, next, legacy);
    crate::commit_retry(tx.commit())
}

/// After a save, best effort (`report_cleanup`): the legacy sidecar `old`
/// the save superseded goes to the trash, and so does each crop of a
/// highlight it deleted whose stamp no merged highlight uses. Assets only.
fn retire(
    store: &Store,
    pdf_name: &str,
    legacy: Option<&FileId>,
    old: Option<&(String, FileRev)>,
    source_key: &str,
    merged: &Merged,
) {
    if let (Some(id), Some((_, rev))) = (legacy, old) {
        let mut cleanup = store.transaction(None);
        cleanup.trash(id, rev.clone(), tine_store::TrashIf::Any);
        report_cleanup(cleanup.commit(), LEGACY_SIDECAR_LEFT);
    }
    let merged_ids: HashSet<&str> = (merged.highlights.iter())
        .map(|item| item.id.as_str())
        .collect();
    let active_stamps: HashSet<i64> = (merged.highlights.iter())
        .filter_map(|item| item.image)
        .collect();
    for item in (merged.disk.iter())
        .filter(|item| item.image.is_some() && !merged_ids.contains(item.id.as_str()))
    {
        let stamp = item.image.unwrap();
        if active_stamps.contains(&stamp) {
            continue;
        }
        let crop_name = format!("{}_{}_{}.png", item.page, item.id, stamp);
        if crop_name.contains('/') || crop_name.contains('\\') {
            continue;
        }
        let crop_rel = if source_key.is_empty() {
            crop_name.clone()
        } else {
            format!("{source_key}/{crop_name}")
        };
        let Ok(crop) = asset(store, &crop_rel) else {
            continue;
        };
        if legacy.is_some_and(|id| !matches!(optional(store, id), Ok(None))) {
            continue;
        }
        // An already-missing crop is the state we want; any other failure
        // leaves an orphan image behind.
        match rollback_pdf_area_file(store, pdf_name, &crop, &item.id, stamp) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                tine_core::diag_line::diagnostic_line(DELETED_CROP_LEFT);
            }
            _ => {}
        }
    }
}

const SIDECAR_CHANGED: &str = "highlight sidecar changed repeatedly during update";

#[cfg(test)]
thread_local! {
    static BEFORE_ANNOTATION_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}
const PAGE_CHANGED: &str = "highlight annotation page changed repeatedly during update";
/// The host operation, as its refusals name it.
const PAGE_UPDATE: &str = "the annotation page update";

fn annotation_steps(
    tx: &mut tine_store::Transaction<'_>,
    annotation: &Annotation,
    doc: &PageDto,
    include_legacy: bool,
) {
    let base = annotation
        .existing
        .as_ref()
        .map_or(SaveBase::CreateNew, |(_, rev)| {
            SaveBase::Existing(rev.clone())
        });
    let mut kinds = vec![tine_store::EditKind::ReplacePage];
    if include_legacy && annotation.legacy.is_some() {
        kinds.push(tine_store::EditKind::DeletePage);
    }
    tx.save_page(&kinds, &annotation.page, base, doc);
    if include_legacy {
        if let Some((id, _, rev)) = &annotation.legacy {
            tx.trash(&id.file(), rev.clone(), tine_store::TrashIf::Any);
        }
    }
}

/// Merge caller highlights by ID against the current sidecar. Changed color,
/// text and image values win locally; unchanged values follow disk. Page and
/// position form one geometry value: changing either locally selects both local
/// values. Unchanged highlights follow disk, including deletion; disk-only
/// additions survive. An edit against a disk deletion, or a deletion against a
/// disk edit, returns a conflict before either artifact is written; retain the
/// local edit for resolution. Returns the committed set for the next baseline.
/// A blank sidecar or valid top-level EDN map is accepted; malformed nonblank
/// EDN refuses. Malformed highlight entries within a valid map are not merged;
/// they are carried through the rewrite unchanged and keep their page blocks.
/// Duplicate IDs are not rejected. Annotation blocks are removed only for
/// highlights deleted by this write.
///
/// With a page host (host-only-writer P3, DESIGN §3.1, §3.4): the pages'
/// unsaved input is saved first (`Input::Flush`); then the sidecar, in its
/// own guarded transaction; then the annotation page, with a migrated
/// legacy page's deletion, as one host operation planned from the page
/// bytes it read, which the host admits only while the page still holds
/// them. The sidecar goes first because it is the highlight data and the
/// page its annotation: a sidecar ahead of its page (a crash or a refused
/// page operation in between) is a page not yet caught up, whose missing
/// blocks the next save adds, while a page ahead of its sidecar would
/// annotate highlights that were never saved. A refused page operation
/// returns its error with the sidecar saved; a retry merges idempotently.
/// With no host: sidecar and annotation page are one guarded transaction,
/// and the legacy page's trash a later best-effort one. Failure can leave
/// disk differences if undo or publication is incomplete; retain local
/// edits and inspect disk.
///
/// After the save, crop/legacy-sidecar trash moves are best effort and do
/// not fail this call; a failed one is reported through `diag_line` and the
/// leftover stays. Cost O(asset entries + sidecar + page + deleted crop
/// bytes + deleted crops × sidecar bytes) per retry, plus graph refresh when
/// the annotation page is absent (up to O(P)).
pub fn write_highlights(
    store: &Store,
    host: Option<&PageHost>,
    pdf_name: &str,
    label: &str,
    highlights: &[Highlight],
    base_highlights: &[Highlight],
) -> io::Result<Vec<Highlight>> {
    let key = pdf::asset_key(pdf_name);
    let primary = asset(store, &format!("{key}.edn"))?;
    let legacy = pdf::legacy_asset_key(pdf_name);
    let legacy_id = (legacy != key && legacy_active(store, pdf_name))
        .then(|| asset(store, &format!("{legacy}.edn")))
        .transpose()?;
    let name = pdf::hls_page_name(&key);
    let old_name = pdf::hls_page_name(&legacy);
    let base: HashMap<&str, &Highlight> =
        base_highlights.iter().map(|h| (h.id.as_str(), h)).collect();
    // The annotation page under both names and formats, unencoded; a page
    // the plan resolves under another spelling is the operation's to save.
    let files: Vec<_> = [&name, &old_name]
        .into_iter()
        .flat_map(|name| ["md", "org"].map(|ext| format!("{name}.{ext}")))
        .filter_map(|rel| store.file_id(Area::Pages, &rel).ok())
        .collect();
    let pages = crate::retained::pages(store, &files);
    let migrate = legacy_id.is_some();
    let read = || {
        let (current, old) = sidecars(store, &primary, legacy_id.as_ref())?;
        let raw = current
            .as_ref()
            .or(old.as_ref())
            .map_or("", |(raw, _)| raw.as_str());
        let merged = merge(raw, highlights, &base)?;
        let next = pdf::write_highlights(&merged.highlights, raw);
        io::Result::Ok((current, old, merged, next))
    };
    let Some(running) = host else {
        // No host: one store transaction under the retained-writer route,
        // as before P3.
        let discover = || Ok(pages.clone());
        return crate::retained::reserved(
            host,
            Input::Flush,
            discover,
            crate::retained::unsaved,
            |_| {
                crate::retry_on_conflict(SIDECAR_CHANGED, || {
                    let (current, old, merged, next) = read()?;
                    let annotation = annotation(store, &name, &old_name, migrate)?;
                    let page_dto = annotation_dto(&annotation, &name, pdf_name, label, &merged);
                    let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
                    let guard = legacy_id.as_ref().zip(old.as_ref());
                    sidecar_steps(&mut tx, &primary, current.as_ref(), &next, guard);
                    if let Some((id, raw, rev)) = &annotation.legacy {
                        if optional(store, &id.file())? != Some((raw.clone(), rev.clone())) {
                            return Ok(None);
                        }
                    }
                    annotation_steps(&mut tx, &annotation, &page_dto, false);
                    if !crate::commit_retry(tx.commit())? {
                        return Ok(None);
                    }
                    let source_key = if old.is_some() { &legacy } else { &key };
                    retire(
                        store,
                        pdf_name,
                        legacy_id.as_ref(),
                        old.as_ref(),
                        source_key,
                        &merged,
                    );
                    if let Some((id, _, rev)) = annotation.legacy {
                        let mut cleanup = store.transaction(Some(tine_store::EditKind::DeletePage));
                        cleanup.trash(&id.file(), rev, tine_store::TrashIf::Any);
                        report_cleanup(cleanup.commit(), LEGACY_PAGE_LEFT);
                    }
                    Ok(Some(merged.highlights))
                })
            },
        );
    };
    // The writer's Flush contract before anything is written: an operation
    // with nothing to write saves the pages' unsaved input.
    let flush = || Ok(None);
    let unsaved = crate::retained::unsaved;
    crate::retained::operation(
        running,
        Input::Flush,
        &pages,
        PAGE_UPDATE,
        PAGE_CHANGED,
        unsaved,
        flush,
    )?;
    let (merged, old) = crate::retry_on_conflict(SIDECAR_CHANGED, || {
        let (current, old, merged, next) = read()?;
        // Validate the page before writing the highlight data.
        let initial = annotation(store, &name, &old_name, migrate)?;
        let doc = annotation_dto(&initial, &name, pdf_name, label, &merged);
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        annotation_steps(&mut tx, &initial, &doc, true);
        match tx.plan() {
            Err(tine_store::Why::Conflict { .. }) => return Ok(None),
            result => {
                result.map_err(crate::why_error)?;
            }
        }
        let guard = legacy_id.as_ref().zip(old.as_ref());
        if !save_sidecar(store, &primary, current.as_ref(), &next, guard)? {
            return Ok(None);
        }
        let saved_rev = FileRev::from_bytes(next.as_bytes());
        let mut remerge = false;
        let plan = || {
            #[cfg(test)]
            BEFORE_ANNOTATION_READ.with(|hook| {
                if let Some(pause) = hook.borrow_mut().take() {
                    pause();
                }
            });
            let annotation = annotation(store, &name, &old_name, migrate)?;
            // Read the page before confirming the sidecar. A saver that has
            // already replaced both is detected here; a subsequent page save
            // invalidates the staged page base at planning or admission.
            if optional(store, &primary)?.as_ref().map(|(_, rev)| rev) != Some(&saved_rev) {
                remerge = true;
                return Ok(None);
            }
            let doc = annotation_dto(&annotation, &name, pdf_name, label, &merged);
            let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
            annotation_steps(&mut tx, &annotation, &doc, true);
            let mut op = crate::retained::plan(tx)?;
            if let Some((source, ..)) = &annotation.legacy {
                op.moved = Some((annotation.page.clone(), source.clone()));
            }
            let unchanged = op.pages.iter().all(|(_, effect, expected)| {
                matches!((effect, expected),
                    (tine_store::HostEffect::Write(new), tine_store::HostExpected::Bytes(old)) if new == old)
            });
            Ok((!unchanged).then_some(op))
        };
        crate::retained::operation(
            running,
            Input::Flush,
            &pages,
            PAGE_UPDATE,
            PAGE_CHANGED,
            unsaved,
            plan,
        )
        .map_err(|error| crate::after_partial("The highlights were saved", error))?;
        Ok((!remerge).then_some((merged, old)))
    })?;
    let source_key = if old.is_some() { &legacy } else { &key };
    retire(
        store,
        pdf_name,
        legacy_id.as_ref(),
        old.as_ref(),
        source_key,
        &merged,
    );
    Ok(merged.highlights)
}
