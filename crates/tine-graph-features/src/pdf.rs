//! PDF sidecars, annotation pages, view state, and cropped images. Guarded
//! overwrites retry a concurrent external revision change at most four times.

use std::collections::HashSet;
use std::io;
use tine_core::model::{Format, PageDto, PageKind};
use tine_core::pdf::{self, Highlight, PdfState};
use tine_store::{Area, Content, FileId, FileRev, PageId, SaveBase, Store, StoreError};

use crate::{is_conflict, store_error, tx_error};

fn asset(store: &Store, rel: &str) -> io::Result<FileId> {
    store.file_id(Area::Assets, rel).map_err(store_error)
}

fn optional(store: &Store, id: &FileId) -> io::Result<Option<(String, FileRev)>> {
    match store.read(id, None) {
        Ok((bytes, rev)) => Ok(Some((
            String::from_utf8(bytes).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stream did not contain valid UTF-8",
                )
            })?,
            rev,
        ))),
        Err(StoreError::NotFound) => Ok(None),
        Err(error) => Err(store_error(error)),
    }
}

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
    let id = match store
        .whole_graph()
        .map_err(|_| io::Error::other("graph unavailable"))?
        .resolve(name, false)
    {
        tine_store::Resolved::Existing { id, .. } | tine_store::Resolved::Absent { id } => id,
        tine_store::Resolved::Alias { .. } => store.as_page(&md).expect("md page"),
    };
    Ok((id, None))
}

fn format(id: &PageId) -> Format {
    if id.as_str().ends_with(".org") {
        Format::Org
    } else {
        Format::Md
    }
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

fn retry_error(operation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        format!("highlight sidecar changed repeatedly during {operation}"),
    )
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

/// Open persisted PDF state, creating only missing OG artifacts. Legacy files
/// remain in place until a highlight write. Cost O(sidecar + annotation page).
pub fn open_pdf(store: &Store, pdf_name: &str, label: &str) -> io::Result<PdfState> {
    let key = pdf::asset_key(pdf_name);
    let (sidecar_id, mut current) = sidecar(store, pdf_name, true)?;
    if let Some((raw, _)) = &current {
        valid_edn(raw)?;
    }
    if current.is_none() {
        let skeleton = pdf::write_highlights(&[], "");
        let mut tx = store.transaction();
        tx.create(&sidecar_id, Content::Bytes(skeleton.clone().into_bytes()));
        let outcome = tx.commit();
        if !is_conflict(&outcome) {
            tx_error(outcome)?;
        }
        current = optional(store, &sidecar_id)?;
    }
    let raw = current.map(|(raw, _)| raw).unwrap_or_default();
    valid_edn(&raw)?;
    let state = pdf::parse_pdf_state(&raw);
    let name = pdf::hls_page_name(&key);
    let (page, present) = page_id(store, &name)?;
    let legacy_name = pdf::hls_page_name(&pdf::legacy_asset_key(pdf_name));
    let legacy_page = if legacy_active(store, pdf_name) && legacy_name != name {
        page_id(store, &legacy_name)?.1.is_some()
    } else {
        false
    };
    if present.is_none() && !legacy_page {
        let doc =
            pdf::hls_page_document_for_format(pdf_name, label, &state.highlights, format(&page));
        let page_dto = dto(&page, &name, &doc);
        let mut tx = store.transaction();
        tx.save_page(&page, SaveBase::CreateNew, &page_dto);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
        }
        tx_error(outcome)?;
    }
    Ok(state)
}

/// Save page and scale while retaining all other sidecar fields. Concurrent
/// external writes are merged on retry, at most four attempts. Cost O(sidecar).
pub fn write_pdf_view_state(
    store: &Store,
    pdf_name: &str,
    page: i64,
    scale: f64,
) -> io::Result<()> {
    for _ in 0..4 {
        let (id, baseline) = sidecar(store, pdf_name, true)?;
        if let Some((raw, _)) = &baseline {
            valid_edn(raw)?;
        }
        let next =
            pdf::write_pdf_view_state(baseline.as_ref().map_or("", |(raw, _)| raw), page, scale)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid PDF view state")
                })?;
        let mut tx = store.transaction();
        match baseline {
            Some((_, rev)) => {
                tx.replace(&id, rev, next.into_bytes());
            }
            None => {
                tx.create(&id, Content::Bytes(next.into_bytes()));
            }
        }
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(retry_error("view-state update"))
}

/// Write a crop under its stable `key/page_id_stamp.png` link. A repeat save
/// replaces that file in place; concurrent external writes are retried four
/// times. Cost O(image bytes) per attempt.
pub fn write_pdf_area_image(
    store: &Store,
    pdf_name: &str,
    page: i64,
    id: &str,
    stamp: i64,
    bytes: &[u8],
) -> io::Result<String> {
    let key = pdf::asset_key(pdf_name);
    let name = format!("{page}_{id}_{stamp}.png");
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad asset name",
        ));
    }
    let rel = format!("{key}/{name}");
    let file_rel = if key.is_empty() {
        name.as_str()
    } else {
        rel.as_str()
    };
    let file = asset(store, file_rel)?;
    for _ in 0..4 {
        let baseline = match store.read(&file, None) {
            Ok((_, rev)) => Some(rev),
            Err(StoreError::NotFound) => None,
            Err(error) => return Err(store_error(error)),
        };
        let mut tx = store.transaction();
        match baseline {
            Some(rev) => {
                tx.replace(&file, rev, bytes.to_vec());
            }
            None => {
                tx.create(&file, Content::Bytes(bytes.to_vec()));
            }
        }
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(rel);
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "PDF area image changed repeatedly during save",
    ))
}

/// Merge highlights with external additions, then commit the sidecar and hls
/// page together. Legacy annotations migrate on write; deleted crops and legacy
/// artifacts go to recoverable trash. Cost O(sidecar + page + deleted crops).
pub fn write_highlights(
    store: &Store,
    pdf_name: &str,
    label: &str,
    highlights: &[Highlight],
    base_ids: &[String],
) -> io::Result<()> {
    let key = pdf::asset_key(pdf_name);
    let primary = asset(store, &format!("{key}.edn"))?;
    let legacy = pdf::legacy_asset_key(pdf_name);
    let legacy_id = (legacy != key && legacy_active(store, pdf_name))
        .then(|| asset(store, &format!("{legacy}.edn")))
        .transpose()?;
    let name = pdf::hls_page_name(&key);
    let old_name = pdf::hls_page_name(&legacy);
    let base: HashSet<&str> = base_ids.iter().map(String::as_str).collect();
    for _ in 0..4 {
        let current = optional(store, &primary)?;
        let old = if current.is_none() {
            legacy_id
                .as_ref()
                .map(|id| optional(store, id))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let raw = current
            .as_ref()
            .or(old.as_ref())
            .map(|(raw, _)| raw.as_str())
            .unwrap_or("");
        valid_edn(raw)?;
        let disk = pdf::parse_highlights(raw);
        let have: HashSet<&str> = highlights.iter().map(|h| h.id.as_str()).collect();
        let mut merged = highlights.to_vec();
        for item in &disk {
            if !have.contains(item.id.as_str()) && !base.contains(item.id.as_str()) {
                merged.push(item.clone());
            }
        }
        let next = pdf::write_highlights(&merged, raw);
        let (mut page, existing) = page_id(store, &name)?;
        let (legacy_page_id, legacy_page) =
            if existing.is_none() && legacy != key && legacy_id.is_some() {
                let (id, value) = page_id(store, &old_name)?;
                (Some(id), value)
            } else {
                (None, None)
            };
        if existing.is_none() && legacy_page.is_some() {
            let ext = if format(legacy_page_id.as_ref().unwrap()) == Format::Org {
                "org"
            } else {
                "md"
            };
            let file = store
                .file_id(Area::Pages, &format!("{name}.{ext}"))
                .map_err(store_error)?;
            page = store.as_page(&file).expect("annotation page");
        }
        let page_raw = existing
            .as_ref()
            .or(legacy_page.as_ref())
            .map(|(raw, _)| raw.as_str());
        if format(&page) == Format::Org
            && page_raw.is_some_and(|raw| !tine_core::org::org_editable(raw))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "org highlight page is read-only (does not round-trip)",
            ));
        }
        let prior = page_raw.map(|raw| parse_doc(raw, format(&page)));
        let doc =
            pdf::merge_hls_page_for_format(prior.as_ref(), pdf_name, label, &merged, format(&page));
        let page_dto = dto(&page, &name, &doc);
        let mut tx = store.transaction();
        match current.as_ref() {
            Some((_, rev)) => {
                tx.replace(&primary, rev.clone(), next.clone().into_bytes());
            }
            None => {
                tx.create(&primary, Content::Bytes(next.clone().into_bytes()));
            }
        }
        if let (Some(id), Some((raw, rev))) = (legacy_id.as_ref(), old.as_ref()) {
            tx.replace(id, rev.clone(), raw.clone().into_bytes());
        }
        if let (Some(id), Some((raw, rev))) = (legacy_page_id.as_ref(), legacy_page.as_ref()) {
            if optional(store, &id.file())?.as_ref() != Some(&(raw.clone(), rev.clone())) {
                continue;
            }
        }
        let base = existing.map_or(SaveBase::CreateNew, |(_, rev)| SaveBase::Existing(rev));
        tx.save_page(&page, base, &page_dto);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        let source_key = if old.is_some() { &legacy } else { &key };
        let merged_ids: HashSet<&str> = merged.iter().map(|item| item.id.as_str()).collect();
        let active_stamps: HashSet<i64> = merged.iter().filter_map(|item| item.image).collect();
        for item in disk
            .iter()
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
            let Ok((_, crop_rev)) = store.read(&crop, None) else {
                continue;
            };
            let Ok(Some((live, _))) = optional(store, &primary) else {
                continue;
            };
            if live != next {
                continue;
            }
            if let (Some(id), Some(expected)) = (legacy_id.as_ref(), old.as_ref()) {
                if optional(store, id).ok().flatten().as_ref() != Some(expected) {
                    continue;
                }
            }
            let mut cleanup = store.transaction();
            cleanup.trash(&crop, crop_rev.clone());
            let Ok(steps) = tx_error(cleanup.commit()) else {
                continue;
            };
            let tine_store::StepResult::Trashed { trashed, .. } = &steps[0] else {
                continue;
            };
            let primary_stable = optional(store, &primary)
                .ok()
                .flatten()
                .is_some_and(|(live, _)| live == next);
            let source_stable = match (legacy_id.as_ref(), old.as_ref()) {
                (Some(id), Some(expected)) => {
                    optional(store, id).ok().flatten().as_ref() == Some(expected)
                }
                _ => true,
            };
            if !primary_stable || !source_stable {
                let mut restore = store.transaction();
                restore.move_file(trashed, crop_rev, &crop, None);
                let _ = restore.commit();
                break;
            }
        }
        if let (Some(id), Some((_, rev))) = (legacy_id.as_ref(), old) {
            let mut cleanup = store.transaction();
            cleanup.trash(id, rev);
            let _ = cleanup.commit();
        }
        if let (Some(id), Some((_, rev))) = (legacy_page_id, legacy_page) {
            let mut cleanup = store.transaction();
            cleanup.trash(&id.file(), rev);
            let _ = cleanup.commit();
        }
        return Ok(());
    }
    Err(retry_error("update"))
}
