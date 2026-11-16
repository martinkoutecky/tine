//! The bytes a page save publishes, as one pure function with two callers:
//! every save ([`Graph::prepare_page_content`], after it has parsed the bytes
//! on disk and given their blocks the cache's runtime ids) and the model's P3
//! check ([`unchanged_save`]: what saving a page loaded from candidate bytes,
//! with no edit, would publish). No lock, no disk, no graph cache.

use super::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    /// Whole-document parses and serializations on this thread (tests reset it).
    static SAVE_WORK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Count one whole-document parse (every store parse of a page text).
pub(super) fn counted_parse() {
    #[cfg(test)]
    SAVE_WORK.with(|w| w.set((w.get().0 + 1, w.get().1)));
}

/// Count one whole-document serialization (a layout-retaining render or a
/// whole-page serializer run).
pub(super) fn counted_serialization() {
    #[cfg(test)]
    SAVE_WORK.with(|w| w.set((w.get().0, w.get().1 + 1)));
}

/// How a page save's content firewalls refuse ([`guarded_save`]).
pub(crate) enum Guard {
    /// The old or the new bytes cannot be parsed safely (size, encoding,
    /// nesting).
    Content(io::Error),
    /// Refusal R-VCS-MARKERS, with its message.
    Markers(String),
    /// The preparation's own refusal or failure.
    Prepare(io::Error),
}

/// Every content firewall of a page save, in order, around `prepare` (which
/// renders the new bytes from the old text, `None` creating): parse
/// validation of the old bytes, VCS conflict markers (unless `resolving`
/// them), parse validation of the new bytes. The one place that decides
/// them, for every page save (`serialize_page_at`) and for the model's P3
/// check ([`unchanged_save`]).
pub(crate) fn guarded_save<B: AsRef<[u8]>, T>(
    old: Option<&[u8]>,
    path: &Path,
    resolving: bool,
    prepare: impl FnOnce(Option<&str>) -> io::Result<(B, T)>,
) -> Result<(B, T), Guard> {
    let text = match old {
        Some(bytes) => {
            validate_parse_bytes_for_path(bytes, path).map_err(Guard::Content)?;
            Some(std::str::from_utf8(bytes).map_err(|error| {
                Guard::Content(io::Error::new(io::ErrorKind::InvalidData, error))
            })?)
        }
        None => None,
    };
    // Refusal R-VCS-MARKERS (docs/storage-contract.md). Threat scenario: a
    // VCS merge by an external writer left unresolved markers; a rewrite
    // would re-indent them and silently lose a side. Every page save reaches
    // this (ordinary, forced, merged and PDF-highlight page saves, and the
    // page host's submits). Only a resolving save passes.
    let found = text
        .map(|text| tine_core::concord_queue::vcs_conflict_markers(text, Format::from_path(path)))
        .unwrap_or_default();
    if !found.is_empty() && !resolving {
        return Err(Guard::Markers(format!(
            "unresolved VCS merge conflict markers ({}); resolve them first",
            found.join(" ")
        )));
    }
    let (new, saved) = prepare(text).map_err(Guard::Prepare)?;
    validate_parse_bytes_for_path(new.as_ref(), path).map_err(Guard::Content)?;
    Ok((new, saved))
}

/// `prepare`, with a parser panic turned into the error every save reports
/// for it ([`ParserPanicked`]).
pub(super) fn parser_unwind<T>(prepare: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(prepare))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, ParserPanicked))?
}

/// The bytes a page whose content is `candidate` at `path` (`owner` is its
/// graph-relative path, giving its blocks a fresh read's runtime ids) would
/// publish if loaded and saved with no edit, or that save's refusal: the
/// content firewalls of [`guarded_save`] (a VCS-marker refusal is
/// `PermissionDenied`, as the save's read-only refusals are) around
/// [`save_content`]. `parsed` is `candidate` parsed by the store
/// (`parse_doc_with_opts`), so a caller that also reads the text for P2
/// parses it once.
///
/// Cost: no parse of `candidate` (the caller's); a validation scan of the
/// candidate and of the result; one serialization and the save's own parse
/// of what it rendered (Markdown layout retention's fallback and header
/// promotion render and parse again); per-block projections. Everything
/// [`save_content`] does.
#[cfg_attr(not(test), allow(dead_code))] // the P3 check's, once it is wired
pub(crate) fn unchanged_save(
    path: &Path,
    owner: Option<&str>,
    candidate: &str,
    parsed: (Document, doc::SerializeOpts),
) -> io::Result<String> {
    let (mut old, opts) = parsed;
    let is_org = matches!(Format::from_path(path), Format::Org);
    if let Some(owner) = owner.filter(|owner| !owner.is_empty()) {
        assign_doc_runtime_ids(&mut old.roots, owner);
    }
    prime_projections(&old);
    // The page an editor holds after loading `candidate`: its DTO, back.
    let page = Document {
        pre_block: old.pre_block.clone(),
        roots: old
            .roots
            .iter()
            .map(|b| tine_core::projection::dto_block_to_doc(&block_to_dto(b), is_org))
            .collect(),
    };
    let save = |_: Option<&str>| {
        parser_unwind(|| save_content(page, path, Some((candidate, &old, &opts))))
    };
    match guarded_save(Some(candidate.as_bytes()), path, false, save) {
        Ok((content, _)) => Ok(content),
        Err(Guard::Content(error) | Guard::Prepare(error)) => Err(error),
        Err(Guard::Markers(why)) => Err(io::Error::new(io::ErrorKind::PermissionDenied, why)),
    }
}

/// Bytes a save writes for `doc` at `path` over `base` (the bytes on disk,
/// their parse with the cache's runtime ids, and their detected layout).
/// Markdown reuses every unchanged block's lines (`layout_retention`), else
/// re-serializes in the file's detected style (no Syncthing churn); equal
/// parses keep the disk bytes and revision (A5); CRLF and lone-CR files keep
/// their terminators (`line_endings`). The bytes re-parse to `doc`, except a
/// document that cannot round-trip (blocks after an unterminated fence): it
/// gets the whole-page serializer's meaning. Returns the bytes and their parse
/// with `doc`'s runtime ids. `path` is the caller's resolved and locked path
/// (M2: never re-resolved here); it names only the format.
pub(super) fn save_content(
    mut doc: Document,
    path: &Path,
    base: Option<(&str, &Document, &doc::SerializeOpts)>,
) -> io::Result<(String, Document)> {
    let existing = base.map(|(source, _, _)| source);
    let old = base.map(|(_, old, _)| old);
    let default_opts = doc::SerializeOpts::default();
    let opts = base.map_or(&default_opts, |(_, _, opts)| opts);
    let dto_is_org = matches!(Format::from_path(path), Format::Org);
    // Data-preservation firewall for page-header properties (GH #163).
    // A frontend/store bug once reclassified a suffix of the page pre-block
    // as the first outline block (`A::` stayed in the header while `B::` and
    // `C::` were serialized as `- B::` / indented continuation text).  The
    // string helper used by the gear panel was correct, so helper tests could
    // not protect the actual DTO -> disk boundary.  No Tine editing command
    // intentionally moves an existing page-header property line into the
    // outline; promotion keeps property lines in the pre-block.  Refuse both
    // normal and force writes that do so, leaving the original bytes intact.
    if let Some(existing_doc) = old {
        // A nonempty disk preamble is authoritative. If a contradictory DTO
        // drops it while presenting a first-root header candidate, refusing
        // the save is safer than either overwriting the preamble or silently
        // keeping the candidate as a bullet. The same validator protects
        // the Store-backed keep-mine path.
        if existing_doc
            .pre_block
            .as_deref()
            .is_some_and(|pre| !pre.is_empty())
            && doc.pre_block.as_deref().unwrap_or("").is_empty()
            && first_root_is_promotable_page_header(&doc)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "refusing to drop an existing page preamble while authoring page-header properties",
            ));
        }
        if let Some(line) = newly_reclassified_page_property_line(existing_doc, &doc, dto_is_org) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("refusing to move page-header property into outline content: {line}"),
            ));
        }
    }
    // Match OG's pre-block serialization decision at one native boundary:
    // a genuinely headerless Markdown page may author a qualifying first
    // root through the ordinary editor, but the persisted/cache shape is an
    // unbulleted page header. Existing nonempty preambles were rejected above.
    if !dto_is_org
        && doc.pre_block.as_deref().unwrap_or("").is_empty()
        && old
            .and_then(|parsed| parsed.pre_block.as_deref())
            .unwrap_or("")
            .is_empty()
    {
        promote_first_root_page_header(&mut doc);
    }
    let (content, mut parsed) = match Format::from_path(path) {
        Format::Md => {
            let retained = existing
                .zip(old)
                .and_then(|(source, old)| layout_retention::serialize(&doc, source, old, opts));
            // Retained lines carry their own terminators; only a
            // whole-page rewrite takes the file's convention.
            let (mut content, reparsed) = retained.unwrap_or_else(|| {
                counted_serialization();
                let content = line_endings::restore(doc::serialize_with(&doc, opts), existing);
                let parsed = parse_doc(path, &content);
                (content, parsed)
            });
            if let Some(e) = existing {
                if e != content && old == Some(&reparsed) {
                    content = e.to_string(); // A5
                }
            }
            (content, reparsed)
        }
        Format::Org => {
            // Corruption firewall: never write a .org file Tine cannot
            // reproduce byte-for-byte. Such a page is served read-only (the
            // editor blocks edits), but defend the write path too — a stale
            // editor or a direct save must not rewrite it. The org model is
            // LF-canonical and its bodies are verbatim, so the serializer is
            // byte-exact up to line terminators, which
            // `layout_retention::restore_org` puts back line by line (K01a).
            let mut round_trip = None;
            if let Some((e, old)) = existing.zip(old) {
                counted_serialization();
                let serialized = tine_core::org::serialize_org_detect(old, Some(e));
                if serialized != tine_core::doc::normalize_line_endings(e) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "org file is read-only (does not round-trip)",
                    ));
                }
                // The serializer reads only what document equality compares
                // (pre-block, raw text, children): an unchanged document's
                // bytes are the firewall's.
                round_trip = (*old == doc).then_some(serialized);
            }
            let content = round_trip.unwrap_or_else(|| {
                counted_serialization();
                tine_core::org::serialize_org_detect(&doc, existing)
            });
            let content = match existing.zip(old) {
                Some((source, old)) => layout_retention::restore_org(content, &doc, source, old),
                None => content,
            };
            let parsed = parse_doc(path, &content);
            (content, parsed)
        }
    };
    carry_saved_runtime_ids(&mut parsed.roots, &doc.roots);
    Ok((content, parsed))
}
