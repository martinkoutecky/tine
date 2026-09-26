//! Recoverable page deletion, rename, stray rescue, and merge. The caller names
//! the operation; this client resolves identities and commits one guarded store
//! transaction. It needs no graph path, lock, cache state, or write protocol.

use std::collections::{HashMap, HashSet};
use std::io;

use tine_core::config::FileNameFormat;
use tine_core::doc;
use tine_core::model::{PageDto, PageKind};
use tine_core::refs;
use tine_store::{
    Area, FileId, FileRev, LoadError, PageId, PageRead, RenameMap, Resolved, SaveBase, SaveOutcome,
    Store, StoreError,
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
    let resolved = store
        .whole_graph()
        .map_err(PageReadError::Load)?
        .resolve(name, kind == PageKind::Journal);
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
            StoreError::InvalidTarget(reason) if reason.starts_with("page source ") => {
                PageReadError::Source(reason)
            }
            other => PageReadError::Store(other),
        })
}

/// Keep-mine reads the current UTF-8 revision and lets the save guard reject
/// later edits. Cost O(page bytes + transaction publication).
pub fn save_page(
    store: &Store,
    id: &PageId,
    page: &PageDto,
    base_rev: Option<String>,
    force: bool,
) -> Result<SaveOutcome, StoreError> {
    if page.guide {
        return Ok(SaveOutcome::GuideEphemeral);
    }
    let base = if force {
        match store.read(&id.file(), None) {
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
    };
    Ok(store.save(id, base, page))
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
    let (bytes, rev) = store.read(file, None).map_err(store_error)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        error(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })?;
    Ok((text, rev))
}

fn encoding(name: &str, fmt: FileNameFormat) -> String {
    // v0.6.5 model.rs `encode_page_name` (6278): slash and underscore order
    // preserves literal triple underscores beside namespace separators.
    match fmt {
        FileNameFormat::Legacy => name.replace('/', "%2F"),
        FileNameFormat::TripleLowbar => name
            .replace("___", "%5F%5F%5F")
            .replace("_/", "%5F/")
            .replace("/_", "/%5F")
            .replace('/', "___"),
    }
}

fn text_file(store: &Store, rel: &str) -> io::Result<FileId> {
    let config = store.config();
    for (area, dir) in [
        (Area::Pages, &config.pages_dir),
        (Area::Journals, &config.journals_dir),
    ] {
        if let Some(tail) = rel.strip_prefix(&format!("{dir}/")) {
            if matches!(
                tail.rsplit_once('.').map(|(_, ext)| ext),
                Some("md" | "org")
            ) {
                return store.file_id(area, tail).map_err(store_error);
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
    for _ in 0..4 {
        let graph = view(store)?;
        let ids = existing(graph.resolve(name, kind == PageKind::Journal));
        validate_target(&ids, expected_path)?;
        let Some(id) = ids.first() else {
            return Ok(());
        };
        let file = id.file();
        let (_, rev) = store.read(&file, None).map_err(store_error)?;
        if expected_rev.is_some_and(|expected| *expected != rev) {
            return Err(error(io::ErrorKind::WouldBlock, "stale page revision"));
        }
        let mut tx = store.transaction();
        tx.trash(&file, rev);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            if expected_rev.is_some() {
                return Err(error(io::ErrorKind::WouldBlock, "stale page revision"));
            }
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(error(
        io::ErrorKind::WouldBlock,
        "page changed repeatedly during delete",
    ))
}

/// Rename a page and its file-backed namespace descendants in one transaction.
/// Pages that explicitly reference a renamed name (`WholeGraph::explicit_referrers`,
/// OG `:block/refs` semantics: `{{query}}` arguments are not references) are
/// rewritten, including `tags::`, aliases and self-references. Non-UTF-8
/// candidates are skipped as in v0.6.5; a non-round-tripping Org referrer
/// refuses the entire rename (H1). Planning costs O(P) plus the referrer query
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
    rename_page_after_inventory(store, old, new, expected_path, || {})
}

fn rename_page_after_inventory(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    after_inventory: impl Fn(),
) -> io::Result<()> {
    let old = old.trim();
    let new = new.trim();
    if new.is_empty() {
        return Err(error(io::ErrorKind::InvalidInput, "empty name"));
    }
    if old.is_empty() || refs::same_page(old, new) {
        return Ok(()); // v0.6.5 model.rs 3549: case-only rename is a no-op.
    }
    for _ in 0..4 {
        let graph = view(store)?;
        let inventory = graph.inventory();
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
            if !existing(graph.resolve(&new_name, false)).is_empty() {
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
                encoding(&new_name, store.config().file_name_format),
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
        if edits.is_empty() {
            return Ok(());
        }
        let mut tx = store.transaction();
        for (id, rev) in edits {
            if let Some(to) = moves.get(&id) {
                tx.move_file(&id.file(), rev, to, Some(&map));
            } else {
                tx.rewrite_refs(&id, rev, &map);
            }
        }
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(error(
        io::ErrorKind::WouldBlock,
        "page changed repeatedly during rename",
    ))
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
        encoding(name, store.config().file_name_format),
        ext
    );
    let to = store.file_id(Area::Pages, &rel).map_err(store_error)?;
    for _ in 0..4 {
        if !existing(view(store)?.resolve(name, false)).is_empty() {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "a page with that name already exists",
            ));
        }
        let (_, rev) = store.read(&src, None).map_err(store_error)?;
        let mut tx = store.transaction();
        tx.move_file(&src, rev, &to, None);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(error(
        io::ErrorKind::WouldBlock,
        "page changed repeatedly during rescue",
    ))
}

/// Merge one source into a survivor, carrying source properties absent in the
/// survivor, then recoverably trash the source in the same commit. Org pairs
/// must round-trip; formats must match. v0.6.5 never rewrites inbound refs in
/// this operation. Cost O(source + survivor bytes and blocks) per try; four
/// complete attempts maximum.
pub fn merge_pages(store: &Store, src_rel: &str, dst_rel: &str) -> io::Result<()> {
    let src = text_file(store, src_rel)?;
    let dst = text_file(store, dst_rel)?;
    if src == dst {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "cannot merge a file into itself",
        ));
    }
    let src_org = src.as_str().ends_with(".org");
    if src_org != dst.as_str().ends_with(".org") {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "files are in different formats",
        ));
    }
    let src_id = store
        .as_page(&src)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let dst_id = store
        .as_page(&dst)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))?;
    for _ in 0..4 {
        let (src_text, src_rev) = read_text(store, &src)?;
        let (dst_text, dst_rev) = read_text(store, &dst)?;
        if src_org
            && (!tine_core::org::org_editable(&src_text)
                || !tine_core::org::org_editable(&dst_text))
        {
            return Err(error(
                io::ErrorKind::PermissionDenied,
                "an org file in this pair does not round-trip; not merging",
            ));
        }
        let source = store.page(&src_id).map_err(store_error)?;
        let mut survivor = store.page(&dst_id).map_err(store_error)?.doc;
        if !src_org {
            if let Some(pre) = source.doc.pre_block.as_deref() {
                let mut dst_pre = survivor.pre_block.clone().unwrap_or_default();
                let keys: HashSet<_> = dst_pre
                    .lines()
                    .filter_map(|line| {
                        doc::parse_property_line(line).map(|(key, _)| key.to_ascii_lowercase())
                    })
                    .collect();
                let extra: Vec<_> = pre
                    .lines()
                    .filter(|line| {
                        doc::parse_property_line(line)
                            .is_some_and(|(key, _)| !keys.contains(&key.to_ascii_lowercase()))
                    })
                    .collect();
                if !extra.is_empty() {
                    if !dst_pre.is_empty() && !dst_pre.ends_with('\n') {
                        dst_pre.push('\n');
                    }
                    for (index, line) in extra.iter().enumerate() {
                        if index != 0 {
                            dst_pre.push('\n');
                        }
                        dst_pre.push_str(line);
                    }
                    survivor.pre_block = Some(dst_pre);
                }
            }
        }
        survivor.blocks.extend(source.doc.blocks);
        let mut tx = store.transaction();
        tx.save_page(&dst_id, SaveBase::Existing(dst_rev), &survivor);
        tx.trash(&src, src_rev);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome)?;
        return Ok(());
    }
    Err(error(
        io::ErrorKind::WouldBlock,
        "pages changed repeatedly during merge",
    ))
}

#[cfg(test)]
#[path = "pages_snapshot_tests.rs"]
mod snapshot_tests;
