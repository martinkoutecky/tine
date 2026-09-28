//! Recoverable page deletion, rename, stray rescue, and merge. The caller names
//! the operation; this client resolves identities and commits one guarded store
//! transaction. It needs no graph path, lock, cache state, or write protocol.

use std::collections::{HashMap, HashSet};
use std::io;

use tine_core::doc;
use tine_core::model::{PageDto, PageKind};
use tine_core::refs;
use tine_store::{
    Area, FileId, FileRev, LoadError, PageId, PageRead, RenameMap, Resolved, SaveBase, SaveOutcome,
    SavePagesOutcome, Store, StoreError,
};

/// A page read or OS source selection failed at the load, identity, or file step.
#[derive(Debug)]
pub enum PageReadError {
    Load(LoadError),
    Store(StoreError),
    EmptyAlias,
    Source(String),
}

/// Resolve a page name or alias and read its current file. Cost O(index lookup + page bytes).
pub fn get_page(
    store: &Store,
    name: &str,
    kind: PageKind,
) -> Result<Option<PageRead>, PageReadError> {
    let resolved = match store.whole_graph() {
        Ok(view) => view.resolve(name, kind == PageKind::Journal),
        Err(LoadError::Failed { .. }) => {
            return store.page_named(name, kind).map_err(PageReadError::Store)
        }
        Err(error) => return Err(PageReadError::Load(error)),
    };
    let id = match resolved {
        Resolved::Existing { id, .. } => id,
        Resolved::Alias { owners } => owners.into_iter().next().ok_or(PageReadError::EmptyAlias)?,
        Resolved::Absent { .. } => return Ok(None),
    };
    match store.page(&id) {
        Ok(read) => Ok(Some(read)),
        Err(StoreError::NotFound) => Ok(None),
        Err(error) => Err(PageReadError::Store(error)),
    }
}

/// Select a source identity and validate the existing OS hand-off path.
/// Cost O(index lookup + path components).
pub fn source_path_for_os_handoff(
    store: &Store,
    name: &str,
    kind: PageKind,
    path: Option<&str>,
) -> Result<std::path::PathBuf, PageReadError> {
    let id = if let Some(path) = path.filter(|path| !path.trim().is_empty()) {
        let config = store.config();
        let file = [
            (Area::Pages, config.pages_dir.as_str()),
            (Area::Journals, config.journals_dir.as_str()),
        ]
        .into_iter()
        .find_map(|(area, dir)| {
            path.strip_prefix(&format!("{dir}/"))
                .and_then(|rel| store.file_id(area, rel).ok())
        })
        .ok_or_else(|| {
            PageReadError::Store(StoreError::InvalidTarget("invalid page path".into()))
        })?;
        store.as_page(&file).ok_or_else(|| {
            PageReadError::Store(StoreError::InvalidTarget("invalid page path".into()))
        })?
    } else {
        match store
            .whole_graph()
            .map_err(PageReadError::Load)?
            .resolve(name, kind == PageKind::Journal)
        {
            Resolved::Existing { id, .. } | Resolved::Absent { id } => id,
            Resolved::Alias { owners } => {
                owners.into_iter().next().ok_or(PageReadError::EmptyAlias)?
            }
        }
    };
    store
        .path_for_os_handoff(&id.file(), true)
        .map_err(|error| match error {
            StoreError::PageSource(reason) => PageReadError::Source(reason),
            other => PageReadError::Store(other),
        })
}

/// Keep-mine reads the current UTF-8 revision and lets the save guard reject
/// later edits. Cost O(page bytes + transaction publication).
pub fn save_page(
    store: &Store,
    kind: tine_store::EditKind,
    id: &PageId,
    page: &PageDto,
    base_rev: Option<String>,
    force: bool,
) -> Result<SaveOutcome, StoreError> {
    if page.guide {
        return Ok(SaveOutcome::GuideEphemeral);
    }
    let base = save_base(store, id, base_rev, force)?;
    Ok(store.save(kind, id, base, page))
}

fn save_base(
    store: &Store,
    id: &PageId,
    base_rev: Option<String>,
    force: bool,
) -> Result<SaveBase, StoreError> {
    Ok(if force {
        match store.read(&id.file(), Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
            Ok((bytes, rev)) => {
                std::str::from_utf8(&bytes).map_err(|_| StoreError::Undecodable)?;
                SaveBase::Existing(rev)
            }
            Err(StoreError::NotFound) => SaveBase::CreateNew,
            Err(error) => return Err(error),
        }
    } else {
        base_rev
            .map(|rev| SaveBase::Existing(rev.into()))
            .unwrap_or(SaveBase::CreateNew)
    })
}

/// Compute each requested base, then save every page in one store transaction.
pub fn save_pages(
    store: &Store,
    entries: &[(
        PageId,
        PageDto,
        Option<String>,
        bool,
        Vec<tine_store::EditKind>,
    )],
) -> Result<SavePagesOutcome, (usize, StoreError)> {
    let mut prepared = Vec::with_capacity(entries.len());
    for (index, (id, page, base_rev, force, kinds)) in entries.iter().enumerate() {
        if page.guide {
            prepared.push((id.clone(), SaveBase::CreateNew, page.clone(), kinds.clone()));
            continue;
        }
        prepared.push((
            id.clone(),
            save_base(store, id, base_rev.clone(), *force).map_err(|error| (index, error))?,
            page.clone(),
            kinds.clone(),
        ));
    }
    Ok(store.save_pages(&prepared))
}

use crate::{is_conflict, store_error, tx_error};

fn error(kind: io::ErrorKind, message: &str) -> io::Error {
    io::Error::new(kind, message)
}

fn view(store: &Store) -> io::Result<tine_store::WholeGraph> {
    store.whole_graph().map_err(|failure| match failure {
        tine_store::LoadError::Failed { reason } => error(io::ErrorKind::Other, &reason),
        tine_store::LoadError::Closed => error(io::ErrorKind::BrokenPipe, "store closed"),
    })
}

fn refreshed_view(store: &Store) -> io::Result<tine_store::WholeGraph> {
    store.scan_refresh().map_err(|failure| match failure {
        tine_store::LoadError::Failed { reason } => error(io::ErrorKind::Other, &reason),
        tine_store::LoadError::Closed => error(io::ErrorKind::BrokenPipe, "store closed"),
    })?;
    view(store)
}

fn existing(target: Resolved) -> Vec<PageId> {
    match target {
        Resolved::Existing { id, mut others } => {
            others.insert(0, id);
            others
        }
        Resolved::Alias { .. } | Resolved::Absent { .. } => Vec::new(),
    }
}

fn physical(target: &Resolved) -> Vec<PageId> {
    match target {
        Resolved::Existing { id, others } => {
            let mut ids = vec![id.clone()];
            ids.extend(others.iter().cloned());
            ids
        }
        Resolved::Alias { .. } | Resolved::Absent { .. } => Vec::new(),
    }
}

fn validate_target(ids: &[PageId], expected_path: Option<&str>) -> io::Result<()> {
    if ids.len() > 1 {
        return Err(error(
            io::ErrorKind::AlreadyExists,
            "multiple files share this page identity; mutation is ambiguous",
        ));
    }
    if let Some(expected) = expected_path.filter(|path| !path.trim().is_empty()) {
        if ids.first().is_none_or(|id| id.as_str() != expected) {
            return Err(error(io::ErrorKind::NotFound, "stale page target"));
        }
    }
    Ok(())
}

fn read_text(store: &Store, file: &FileId) -> io::Result<(String, FileRev)> {
    crate::parsed_text::read(store, file)
}

fn text_file(store: &Store, rel: &str) -> io::Result<FileId> {
    let config = store.config();
    for (area, dir) in [
        (Area::Pages, &config.pages_dir),
        (Area::Journals, &config.journals_dir),
    ] {
        if let Some(tail) = rel.strip_prefix(&format!("{dir}/")) {
            if let Ok(file) = store.file_id(area, tail) {
                if tine_store::is_graph_text(&file) {
                    return Ok(file);
                }
            }
        }
    }
    Err(error(io::ErrorKind::InvalidInput, "invalid file path"))
}

/// Delete one named page or journal into recoverable trash. A supplied revision
/// pins the displayed version; without one the current version is read and
/// guarded. An absent reference-only page is a no-op. Cost O(P + file bytes)
/// per try; a conflict is replanned at most four times.
pub fn delete_page_expected(
    store: &Store,
    name: &str,
    kind: PageKind,
    expected_path: Option<&str>,
    expected_rev: Option<&FileRev>,
) -> io::Result<()> {
    crate::retry_on_conflict("page changed repeatedly during delete", || {
        let graph = refreshed_view(store)?;
        let ids = existing(graph.resolve(name, kind == PageKind::Journal));
        validate_target(&ids, expected_path)?;
        let Some(id) = ids.first() else {
            return Ok(Some(()));
        };
        let file = id.file();
        let (_, rev) = store
            .read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES))
            .map_err(store_error)?;
        if expected_rev.is_some_and(|expected| *expected != rev) {
            return Err(error(io::ErrorKind::WouldBlock, "stale page revision"));
        }
        let mut tx = store.transaction(Some(tine_store::EditKind::DeletePage));
        tx.trash(&file, rev);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            if expected_rev.is_some() {
                return Err(error(io::ErrorKind::WouldBlock, "stale page revision"));
            }
            return Ok(None);
        }
        tx_error(outcome)?;
        Ok(Some(()))
    })
}

/// Rename a page and its file-backed namespace descendants in one transaction.
/// Pages that explicitly reference a renamed name (`WholeGraph::explicit_referrers`,
/// OG `:block/refs` semantics: `{{query}}` arguments are not references) are
/// rewritten, including `tags::`, aliases and self-references. Non-UTF-8
/// candidates are skipped as in v0.6.5; a non-round-tripping Org referrer
/// refuses the entire rename (H1). A target name another page file already
/// owns refuses with `AlreadyExists` and writes nothing; [`rename_or_merge_page`]
/// is the confirmed alternative. Planning costs O(P) plus the referrer query
/// and O(referrer bytes) reads, followed by O(touched bytes) commit. A rename
/// touching N files takes N+1 sorted path locks for one move; namespace moves
/// take one additional destination lock per moved file. Conflict retry: four
/// complete plans maximum.
pub fn rename_page_expected(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
) -> io::Result<()> {
    rename_page_after_inventory(
        store,
        old,
        new,
        expected_path,
        None,
        #[cfg(test)]
        || {},
    )
}

/// [`rename_page_expected`] when `merge_into` is `None`. Otherwise rename onto
/// the existing page file `merge_into` the user confirmed, as OG `merge-pages!`
/// (master GH #327): the source's blocks append to that survivor, its property
/// lines join the survivor's header (`alias::` values are united; the survivor
/// wins another clash, and the source `title::` and clashing lines move with
/// the blocks as one ordinary block), the source file goes to graph trash, and
/// references and namespace descendants are renamed exactly as by
/// [`rename_page_expected`]. Everything is one guarded transaction. It refuses
/// before any write when `merge_into` no longer solely owns the new name, the
/// formats differ, an Org file does not round-trip, or a descendant's target
/// exists. Cost: the rename's, plus O(source + survivor bytes and blocks).
pub fn rename_or_merge_page(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    merge_into: Option<&str>,
) -> io::Result<()> {
    rename_page_after_inventory(
        store,
        old,
        new,
        expected_path,
        merge_into,
        #[cfg(test)]
        || {},
    )
}

fn rename_page_after_inventory(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    merge_into: Option<&str>,
    #[cfg(test)] after_inventory: impl Fn(),
) -> io::Result<()> {
    let old = old.trim();
    let new = new.trim();
    if new.is_empty() {
        return Err(error(io::ErrorKind::InvalidInput, "empty name"));
    }
    if old.is_empty() || refs::same_page(old, new) {
        return Ok(()); // v0.6.5 model.rs 3549: case-only rename is a no-op.
    }
    crate::retry_on_conflict("page changed repeatedly during rename", || {
        let graph = refreshed_view(store)?;
        let inventory = graph.inventory();
        #[cfg(test)]
        after_inventory();
        let source = existing(graph.resolve(old, false));
        validate_target(&source, expected_path)?;
        let old_key = refs::normalize(old);
        let prefix = format!("{old_key}/");
        let mut pairs = Vec::new();
        let mut moves = HashMap::<PageId, FileId>::new();
        let mut destinations = HashSet::new();
        let mut identities = HashSet::new();
        let mut primary_is_file = false;
        let mut merge = None;
        for entry in &inventory.0 {
            if entry.is_journal {
                continue;
            }
            let ids = physical(&entry.target);
            if ids.is_empty() {
                continue;
            }
            let key = refs::normalize(&entry.name);
            let primary = key == old_key;
            if !primary && !key.starts_with(&prefix) {
                continue;
            }
            let new_name = if primary {
                new.to_owned()
            } else {
                format!(
                    "{new}{}",
                    entry
                        .name
                        .chars()
                        .skip(old.chars().count())
                        .collect::<String>()
                )
            };
            if ids.len() > 1 {
                return Err(error(
                    io::ErrorKind::AlreadyExists,
                    "multiple files share this page identity; mutation is ambiguous",
                ));
            }
            let id = ids[0].clone();
            let taken = existing(graph.resolve(&new_name, false));
            if let (true, Some(into), [survivor]) = (primary, merge_into, taken.as_slice()) {
                if survivor.as_str() == into && *survivor != id {
                    identities.insert(refs::normalize(&new_name));
                    merge = Some((id, survivor.clone()));
                    primary_is_file = true;
                    pairs.push((entry.name.clone(), new_name));
                    continue;
                }
            }
            if !taken.is_empty() {
                return Err(error(
                    io::ErrorKind::AlreadyExists,
                    "target page identity already exists elsewhere in the graph",
                ));
            }
            let ext = if id.as_str().ends_with(".org") {
                "org"
            } else {
                "md"
            };
            let rel = format!(
                "{}.{}",
                tine_core::model::encode_page_name(&new_name, store.config().file_name_format),
                ext
            );
            let to = store.file_id(Area::Pages, &rel).map_err(store_error)?;
            if !destinations.insert(to.as_str().to_owned())
                || !identities.insert(refs::normalize(&new_name))
            {
                return Err(error(
                    io::ErrorKind::AlreadyExists,
                    "multiple pages map to the same rename target",
                ));
            }
            if to == id.file() {
                return Err(error(io::ErrorKind::AlreadyExists, "target page exists"));
            }
            if primary {
                primary_is_file = true;
            }
            pairs.push((entry.name.clone(), new_name));
            moves.insert(id, to);
        }
        if merge_into.is_some() && merge.is_none() {
            return Err(error(
                io::ErrorKind::NotFound,
                "the page to merge into no longer owns that name",
            ));
        }
        // v0.6.5 model.rs 3654: reference-only pages have no move, but refs change.
        if !primary_is_file {
            pairs.push((old.to_owned(), new.to_owned()));
        }
        let map = RenameMap(pairs);
        let lookup: HashMap<_, _> = map
            .0
            .iter()
            .map(|(from, to)| (refs::normalize(from), to.clone()))
            .collect();
        // v0.6.5 model.rs 3681-3690 (warm index) and OG `:block/refs`: only
        // pages that explicitly reference a renamed name are rewritten, plus
        // every moved page. A name mentioned only inside `{{query}}` stays.
        let olds: Vec<String> = map.0.iter().map(|(from, _)| from.clone()).collect();
        let mut candidates: Vec<PageId> = graph.explicit_referrers(&olds);
        candidates.extend(moves.keys().cloned());
        candidates.retain(|id| {
            merge
                .as_ref()
                .is_none_or(|(src, dst)| id != src && id != dst)
        });
        candidates.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        candidates.dedup();
        let mut edits = Vec::new();
        for id in candidates {
            let file = id.file();
            let (content, rev) = match read_text(store, &file) {
                Ok(value) => value,
                Err(_) => continue, // v0.6.5 model.rs 3699 skips unreadable candidates.
            };
            let org = id.as_str().ends_with(".org");
            let updated = refs::rename_tags_property_multi(
                &refs::rename_refs_multi(&content, &lookup, org),
                &lookup,
                org,
            );
            if org && updated != content && !tine_core::org::org_editable(&content) {
                let display = store
                    .path_for_os_handoff(&file, false)
                    .map_err(store_error)?;
                return Err(error(
                    io::ErrorKind::PermissionDenied,
                    &format!(
                        "cannot rename: {} is a read-only .org file (does not round-trip)",
                        display.display()
                    ),
                ));
            }
            if moves.contains_key(&id) || updated != content {
                edits.push((id, rev));
            }
        }
        let merged = match &merge {
            Some((src, dst)) => Some(merged_survivor(store, src, dst, Some(&lookup))?),
            None => None,
        };
        if edits.is_empty() && merged.is_none() {
            return Ok(Some(()));
        }
        let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
        if let (Some((src, dst)), Some(survivor)) = (&merge, merged) {
            let kinds = [
                tine_store::EditKind::RenamePage,
                tine_store::EditKind::InsertBlocks,
            ];
            tx.save_page(
                &kinds,
                dst,
                SaveBase::Existing(survivor.dst_rev),
                &survivor.doc,
            );
            tx.trash(&src.file(), survivor.src_rev);
        }
        for (id, rev) in edits {
            if let Some(to) = moves.get(&id) {
                tx.move_file(&id.file(), rev, to, Some(&map));
            } else {
                tx.rewrite_refs(&id, rev, &map);
            }
        }
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

/// Rescue a stray page or journal file into a uniquely named normal page.
/// Its bytes are unchanged and inbound references are not rewritten, matching
/// v0.6.5 model.rs 1779. Cost O(P + source bytes) per try; four tries maximum.
pub fn rename_file_to_page(store: &Store, src_rel: &str, new_name: &str) -> io::Result<()> {
    let name = new_name.trim();
    if name.is_empty() {
        return Err(error(io::ErrorKind::InvalidInput, "empty page name"));
    }
    let src = text_file(store, src_rel)?;
    let ext = if src.as_str().ends_with(".org") {
        "org"
    } else {
        "md"
    };
    let rel = format!(
        "{}.{}",
        tine_core::model::encode_page_name(name, store.config().file_name_format),
        ext
    );
    let to = store.file_id(Area::Pages, &rel).map_err(store_error)?;
    crate::retry_on_conflict("page changed repeatedly during rescue", || {
        if !existing(refreshed_view(store)?.resolve(name, false)).is_empty() {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "a page with that name already exists",
            ));
        }
        let (_, rev) = store
            .read(&src, Some(tine_store::PARSE_INPUT_MAX_BYTES))
            .map_err(store_error)?;
        let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
        tx.move_file(&src, rev, &to, None);
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

/// Merge one source into a survivor, as [`rename_or_merge_page`] merges pages
/// but without renaming anything, then recoverably trash the source in the
/// same commit. Org pairs must round-trip; formats must match. v0.6.5 never
/// rewrites inbound refs in this operation. Cost O(source + survivor bytes and
/// blocks) per try; four complete attempts maximum.
pub fn merge_pages(store: &Store, src_rel: &str, dst_rel: &str) -> io::Result<()> {
    let src = text_file(store, src_rel)?;
    let dst = text_file(store, dst_rel)?;
    if src == dst {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "cannot merge a file into itself",
        ));
    }
    let src_id = store
        .as_page(&src)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let dst_id = store
        .as_page(&dst)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    crate::retry_on_conflict("pages changed repeatedly during merge", || {
        let survivor = merged_survivor(store, &src_id, &dst_id, None)?;
        let mut tx = store.transaction(Some(tine_store::EditKind::InsertBlocks));
        tx.save_page(
            &[
                tine_store::EditKind::InsertBlocks,
                tine_store::EditKind::DeletePage,
            ],
            &dst_id,
            SaveBase::Existing(survivor.dst_rev),
            &survivor.doc,
        );
        tx.trash(&src, survivor.src_rev);
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    })
}

fn lines<'a>(mut parts: impl Iterator<Item = &'a str>) -> String {
    let first = parts.next().unwrap_or_default().to_owned();
    parts.fold(first, |text, line| text + "\n" + line)
}

struct Survivor {
    doc: PageDto,
    src_rev: FileRev,
    dst_rev: FileRev,
}

/// The survivor page after appending `src`, with `renames` applied to its text.
/// OG `merge-pages!` moves the source's property block with its blocks; here a
/// source property the survivor lacks joins its header, `alias::` values are
/// united, and the rest (a clash, the source `title::`, free text) moves as one
/// ordinary block, so the source identity never renames the survivor. Source
/// aliases join the survivor's only on a rename-merge (`renames`), whose
/// contract is that every old name keeps resolving; a plain merge keeps them
/// verbatim in the moved block (I-4).
fn merged_survivor(
    store: &Store,
    src: &PageId,
    dst: &PageId,
    renames: Option<&HashMap<String, String>>,
) -> io::Result<Survivor> {
    let org = src.as_str().ends_with(".org");
    if org != dst.as_str().ends_with(".org") {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "files are in different formats",
        ));
    }
    let (src_text, src_rev) = read_text(store, &src.file())?;
    let (dst_text, dst_rev) = read_text(store, &dst.file())?;
    if org && (!tine_core::org::org_editable(&src_text) || !tine_core::org::org_editable(&dst_text))
    {
        return Err(error(
            io::ErrorKind::PermissionDenied,
            "an org file in this pair does not round-trip; not merging",
        ));
    }
    let source = store.page(src).map_err(store_error)?.doc;
    let mut doc = store.page(dst).map_err(store_error)?.doc;
    if let (false, Some(pre)) = (org, source.pre_block.as_deref()) {
        let mut header: Vec<String> = doc
            .pre_block
            .as_deref()
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        let mut moved = Vec::new();
        for line in pre.lines() {
            let Some((key, value)) = doc::parse_property_line(line) else {
                moved.push(line);
                continue;
            };
            let key = key.to_ascii_lowercase();
            let clash = header.iter().position(|kept| {
                doc::parse_property_line(kept).is_some_and(|(k, _)| k.eq_ignore_ascii_case(&key))
            });
            match clash {
                None if key != "title" => header.push(line.to_owned()),
                Some(at) if key == "alias" && renames.is_some() => {
                    let kept = header[at].clone();
                    let known: HashSet<String> = doc::parse_property_line(&kept)
                        .map(|(_, v)| v.split(',').map(refs::normalize).collect())
                        .unwrap_or_default();
                    let extra: Vec<&str> = value
                        .split(',')
                        .map(str::trim)
                        .filter(|alias| {
                            !alias.is_empty() && !known.contains(&refs::normalize(alias))
                        })
                        .collect();
                    if !extra.is_empty() {
                        header[at] = extra
                            .iter()
                            .fold(kept.trim_end().to_owned(), |line, alias| {
                                format!("{line}, {alias}")
                            });
                    }
                }
                _ => moved.push(line),
            }
        }
        if !header.is_empty() {
            doc.pre_block = Some(lines(header.iter().map(String::as_str)));
        }
        if moved.iter().any(|line| !line.trim().is_empty()) {
            doc.blocks.push(tine_core::model::BlockDto {
                raw: lines(moved.into_iter()),
                ..Default::default()
            });
        }
    }
    doc.blocks.extend(source.blocks);
    if let Some(renames) = renames {
        fn rewrite(raw: &mut String, renames: &HashMap<String, String>, org: bool) {
            *raw = refs::rename_tags_property_multi(
                &refs::rename_refs_multi(raw, renames, org),
                renames,
                org,
            );
        }
        fn walk(
            blocks: &mut [tine_core::model::BlockDto],
            renames: &HashMap<String, String>,
            org: bool,
        ) {
            for block in blocks {
                rewrite(&mut block.raw, renames, org);
                walk(&mut block.children, renames, org);
            }
        }
        if let Some(pre) = doc.pre_block.as_mut() {
            rewrite(pre, renames, org);
        }
        walk(&mut doc.blocks, renames, org);
    }
    Ok(Survivor {
        doc,
        src_rev,
        dst_rev,
    })
}

#[cfg(test)]
#[path = "pages_snapshot_tests.rs"]
mod snapshot_tests;
