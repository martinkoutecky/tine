use crate::date::JournalDate;
use crate::doc::{property_key_norm, DocBlock, Document};
use crate::model::{Format, PageEntry, PageKind};
use crate::query::{
    run_parser_sparse_task_query_bounded, sparse_task_query_eligibility,
    ApplicationSparseQueryPage, BoundedGroups, ParserSparseQueryCandidate,
};
use fs2::FileExt as _;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tine_storage::sqlite::{
    PhysicalBlock, PhysicalEntityId, PhysicalGraphProjectionChange,
    PhysicalGraphProjectionDatabase, PhysicalGraphProjectionSourceRevision, PhysicalPage,
    PhysicalProperty, PhysicalReferencePosting, PhysicalReferenceTarget, PhysicalTask,
};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

type PageSnapshot = Arc<Vec<(PageEntry, Arc<Document>)>>;
type PageRevisions = Arc<HashMap<PathBuf, String>>;

#[cfg(test)]
static PHYSICAL_PAGE_LOWERINGS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
enum PageDelta {
    Replace(PageEntry, Arc<Document>, String),
    Delete(PageEntry),
}

#[derive(Default)]
struct PendingProjection {
    full: Option<(u64, PageSnapshot, PageRevisions)>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
    latest_generation: u64,
    stop: bool,
}

struct ProjectionShared {
    path: PathBuf,
    pending: Mutex<PendingProjection>,
    changed: Condvar,
    ready: AtomicBool,
    ready_generation: AtomicU64,
    reader: Mutex<Option<PhysicalGraphProjectionDatabase>>,
    #[cfg(test)]
    indexed_reads: AtomicU64,
    #[cfg(test)]
    referenced_name_reads: AtomicU64,
    #[cfg(test)]
    fuzzy_candidate_reads: AtomicU64,
}

/// Direct Files' disposable parser-fact projection.
///
/// The foreground only publishes already-parsed `Arc<Document>` snapshots into
/// a coalescing page map. One worker owns SQLite, so an editor save never waits
/// for schema work, SQL, disk flushes, or a graph-sized rebuild. Read paths may
/// use the database only at the exact current cache generation.
pub(crate) struct DirectProjection {
    shared: Arc<ProjectionShared>,
}

impl DirectProjection {
    pub(crate) fn start(path: PathBuf) -> std::io::Result<Self> {
        let shared = Arc::new(ProjectionShared {
            path,
            pending: Mutex::new(PendingProjection::default()),
            changed: Condvar::new(),
            ready: AtomicBool::new(false),
            ready_generation: AtomicU64::new(0),
            reader: Mutex::new(None),
            #[cfg(test)]
            indexed_reads: AtomicU64::new(0),
            #[cfg(test)]
            referenced_name_reads: AtomicU64::new(0),
            #[cfg(test)]
            fuzzy_candidate_reads: AtomicU64::new(0),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("tine-direct-projection".into())
            .spawn(move || projection_worker(worker))?;
        Ok(Self { shared })
    }

    pub(crate) fn enqueue_full(
        &self,
        generation: u64,
        pages: PageSnapshot,
        revisions: PageRevisions,
    ) {
        self.shared.ready.store(false, Ordering::Release);
        let mut pending = self.shared.pending.lock().unwrap();
        pending.full = Some((generation, pages, revisions));
        pending.deltas.clear();
        pending.latest_generation = generation;
        self.shared.changed.notify_one();
    }

    pub(crate) fn enqueue_replace(
        &self,
        generation: u64,
        entry: PageEntry,
        document: Arc<Document>,
        revision: String,
    ) {
        self.enqueue_delta(generation, PageDelta::Replace(entry, document, revision));
    }

    pub(crate) fn enqueue_delete(&self, generation: u64, entry: PageEntry) {
        self.enqueue_delta(generation, PageDelta::Delete(entry));
    }

    fn enqueue_delta(&self, generation: u64, delta: PageDelta) {
        self.shared.ready.store(false, Ordering::Release);
        let key = match &delta {
            PageDelta::Replace(entry, _, _) | PageDelta::Delete(entry) => entry.rel_path.clone(),
        };
        let mut pending = self.shared.pending.lock().unwrap();
        pending.deltas.insert(key, (generation, delta));
        pending.latest_generation = pending.latest_generation.max(generation);
        self.shared.changed.notify_one();
    }

    pub(crate) fn mark_stale(&self) {
        self.shared.ready.store(false, Ordering::Release);
    }

    pub(crate) fn sparse_task_query(
        &self,
        graph_root: &Path,
        cache_generation: u64,
        pages: &[(PageEntry, Arc<Document>)],
        query_src: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> Option<BoundedGroups> {
        let eligibility = sparse_task_query_eligibility(query_src)?;
        if !self.shared.ready.load(Ordering::Acquire)
            || self.shared.ready_generation.load(Ordering::Acquire) != cache_generation
        {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut by_block = BTreeMap::new();
        let uses_recency = eligibility.uses_recency;
        const BATCH: usize = 1024;
        for marker in eligibility.markers {
            let mut after = None;
            loop {
                let rows = read
                    .task_candidate_locators_after(&marker, after, BATCH)
                    .ok()?;
                let count = rows.len();
                for row in rows {
                    after = Some((row.page_id, row.block_id));
                    by_block.entry(row.block_id).or_insert(row);
                }
                if count < BATCH {
                    break;
                }
            }
        }
        if self.shared.ready_generation.load(Ordering::Acquire) != cache_generation
            || !self.shared.ready.load(Ordering::Acquire)
        {
            return None;
        }
        let mut page_recencies = HashMap::<String, i64>::new();
        struct CandidateMetadata {
            block_id: String,
            parent_identity: Option<String>,
            order: Vec<String>,
            page: ApplicationSparseQueryPage,
        }
        let metadata = by_block
            .into_values()
            .map(|row| {
                let recency = if uses_recency {
                    *page_recencies
                        .entry(row.page_path.clone())
                        .or_insert_with(|| {
                            page_recency(
                                graph_root,
                                &row.page_name,
                                &row.page_path,
                                row.page_text_kind,
                            )
                        })
                } else {
                    i64::MIN
                };
                CandidateMetadata {
                    block_id: Uuid::from_bytes(row.block_id).to_string(),
                    parent_identity: row.parent.map(|id| Uuid::from_bytes(id).to_string()),
                    order: vec![row.order, Uuid::from_bytes(row.block_id).to_string()],
                    page: ApplicationSparseQueryPage {
                        name: row.page_name,
                        path: row.page_path.clone(),
                        kind: page_kind_from_sql(row.page_text_kind)?,
                        is_org: Format::from_path(Path::new(&row.page_path)) == Format::Org,
                        recency,
                    },
                }
                .into()
            })
            .collect::<Option<Vec<_>>>()?;
        let documents = pages
            .iter()
            .map(|(entry, document)| (entry.rel_path.as_str(), document.as_ref()))
            .collect::<HashMap<_, _>>();
        let candidates = metadata
            .iter()
            .map(|candidate| {
                let document = documents.get(candidate.page.path.as_str())?;
                let block = block_at_order(&document.roots, &candidate.order[0])?;
                (block.uuid == candidate.block_id).then_some(ParserSparseQueryCandidate {
                    block,
                    identity: &candidate.block_id,
                    page: &candidate.page,
                    parent_identity: candidate.parent_identity.as_deref(),
                    dfs_order: &candidate.order,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let result =
            run_parser_sparse_task_query_bounded(&candidates, query_src, max_rows, max_bytes)
                .ok()?;
        let current = (self.shared.ready.load(Ordering::Acquire)
            && self.shared.ready_generation.load(Ordering::Acquire) == cache_generation)
            .then_some(result);
        #[cfg(test)]
        if current.is_some() {
            self.shared.indexed_reads.fetch_add(1, Ordering::Relaxed);
        }
        current
    }

    pub(crate) fn referenced_page_names(&self, cache_generation: u64) -> Option<Vec<String>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut after: Option<(String, String, String, [u8; 16])> = None;
        let mut names = std::collections::HashMap::<String, String>::new();
        const BATCH: usize = 1024;
        loop {
            let rows = read
                .navigation_reference_names_after(
                    after.as_ref().map(|(path, raw, normalized, id)| {
                        (path.as_str(), raw.as_str(), normalized.as_str(), id)
                    }),
                    BATCH,
                )
                .ok()?;
            let count = rows.len();
            for row in rows {
                after = Some((
                    row.owner_path,
                    row.raw_name.clone(),
                    row.normalized_name,
                    row.source_page_id,
                ));
                names
                    .entry(crate::refs::page_key(&row.raw_name))
                    .or_insert(row.raw_name);
            }
            if count < BATCH {
                break;
            }
        }
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut names = names.into_values().collect::<Vec<_>>();
        names.sort_by_key(|name| crate::refs::page_key(name));
        #[cfg(test)]
        self.shared
            .referenced_name_reads
            .fetch_add(1, Ordering::Relaxed);
        Some(names)
    }

    pub(crate) fn fuzzy_candidate_paths(
        &self,
        cache_generation: u64,
        normalized_needle: &str,
    ) -> Option<std::collections::HashSet<String>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut after = None;
        let mut paths = std::collections::HashSet::new();
        const BATCH: usize = 1024;
        loop {
            let rows = read
                .fuzzy_subsequence_candidate_pages_after(normalized_needle, after, BATCH)
                .ok()?;
            let count = rows.len();
            for row in rows {
                after = Some(row.page_id);
                paths.insert(row.path);
            }
            if count < BATCH {
                break;
            }
        }
        let current = self.ready_at(cache_generation).then_some(paths);
        #[cfg(test)]
        if current.is_some() {
            self.shared
                .fuzzy_candidate_reads
                .fetch_add(1, Ordering::Relaxed);
        }
        current
    }

    pub(crate) fn ready_at(&self, generation: u64) -> bool {
        self.shared.ready.load(Ordering::Acquire)
            && self.shared.ready_generation.load(Ordering::Acquire) == generation
    }

    #[cfg(test)]
    pub(crate) fn indexed_reads(&self) -> u64 {
        self.shared.indexed_reads.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn referenced_name_reads(&self) -> u64 {
        self.shared.referenced_name_reads.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn fuzzy_candidate_reads(&self) -> u64 {
        self.shared.fuzzy_candidate_reads.load(Ordering::Relaxed)
    }
}

fn block_at_order<'a>(roots: &'a [DocBlock], order: &str) -> Option<&'a DocBlock> {
    let mut siblings = roots;
    let mut found = None;
    for component in order.split('/') {
        if component.len() != 8 {
            return None;
        }
        let index = usize::try_from(u32::from_str_radix(component, 16).ok()?).ok()?;
        let block = siblings.get(index)?;
        found = Some(block);
        siblings = &block.children;
    }
    found
}

impl Drop for DirectProjection {
    fn drop(&mut self) {
        let mut pending = self.shared.pending.lock().unwrap();
        pending.stop = true;
        self.shared.changed.notify_one();
    }
}

fn projection_worker(shared: Arc<ProjectionShared>) {
    let Some(parent) = shared.path.parent() else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        eprintln!("[tine] Direct Files SQLite projection disabled: create directory: {error}");
        return;
    }
    let lease_path = shared.path.with_extension("sqlite.writer.lock");
    let lease = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lease_path)
        .and_then(|file| {
            file.try_lock_exclusive()?;
            Ok(file)
        }) {
        Ok(lease) => lease,
        Err(error) => {
            eprintln!(
                "[tine] Direct Files SQLite projection unavailable; another graph instance owns it or its lease cannot be opened: {error}"
            );
            return;
        }
    };
    let mut database = match open_projection_database(&shared.path) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("[tine] Direct Files SQLite projection disabled: {error}");
            return;
        }
    };
    // The lock file is app-private disposable state. Retain its exclusive lock
    // for the complete writer lifetime so another Graph instance cannot replace
    // this database's facts behind a locally-ready generation watermark.
    let _lease = lease;
    let mut requires_full_rebuild = false;
    loop {
        let (full, deltas, latest_generation) = {
            let mut pending = shared.pending.lock().unwrap();
            while pending.full.is_none() && pending.deltas.is_empty() && !pending.stop {
                pending = shared.changed.wait(pending).unwrap();
            }
            if pending.stop {
                return;
            }
            (
                pending.full.take(),
                std::mem::take(&mut pending.deltas),
                pending.latest_generation,
            )
        };
        let had_full = full.is_some();
        let applied = if requires_full_rebuild && !had_full {
            Err("a prior projection failure requires a complete parser snapshot".into())
        } else {
            apply_pending(&mut database, full, deltas)
        };
        if let Err(error) = applied {
            requires_full_rebuild = true;
            shared.ready.store(false, Ordering::Release);
            eprintln!(
                "[tine] Direct Files SQLite projection is stale; using parser fallback: {error}"
            );
            continue;
        }
        if had_full {
            requires_full_rebuild = false;
        }
        let pending = shared.pending.lock().unwrap();
        if pending.full.is_none()
            && pending.deltas.is_empty()
            && pending.latest_generation == latest_generation
        {
            shared
                .ready_generation
                .store(latest_generation, Ordering::Release);
            shared.ready.store(true, Ordering::Release);
        }
    }
}

fn open_projection_database(
    path: &Path,
) -> Result<PhysicalGraphProjectionDatabase, tine_storage::sqlite::MaterializationError> {
    let database = PhysicalGraphProjectionDatabase::open_writable(path)?;
    if database.validate_schema().is_ok() && database.quick_check().is_ok() {
        return Ok(database);
    }
    if database.initialize_schema().is_ok()
        && database.validate_schema().is_ok()
        && database.quick_check().is_ok()
    {
        return Ok(database);
    }
    drop(database);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let database = PhysicalGraphProjectionDatabase::open_writable(path)?;
    database.initialize_schema()?;
    database.validate_schema()?;
    Ok(database)
}

fn apply_pending(
    database: &mut PhysicalGraphProjectionDatabase,
    full: Option<(u64, PageSnapshot, PageRevisions)>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
) -> Result<(), String> {
    if let Some((_, pages, revisions)) = full {
        let sources = pages
            .iter()
            .map(|(entry, _)| {
                Ok(PhysicalGraphProjectionSourceRevision {
                    page_id: page_id(&entry.rel_path),
                    revision: revisions.get(&entry.path).cloned().ok_or_else(|| {
                        format!(
                            "parsed page has no exact source revision: {}",
                            entry.rel_path
                        )
                    })?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let source_delta = database
            .source_delta(&sources)
            .map_err(|error| error.to_string())?;
        let replacements_needed = source_delta
            .replacements
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let lowered = pages
            .iter()
            .filter(|(entry, _)| replacements_needed.contains(&page_id(&entry.rel_path)))
            .map(|(entry, document)| physical_page(entry, document))
            .collect::<Result<Vec<_>, _>>()?;
        let mut replacements = Vec::with_capacity(lowered.len());
        let mut reference_postings = Vec::new();
        for (page, mut postings) in lowered {
            replacements.push(page);
            reference_postings.append(&mut postings);
        }
        let replacement_sources = sources
            .into_iter()
            .filter(|source| replacements_needed.contains(&source.page_id))
            .collect::<Vec<_>>();
        database
            .apply_with_source_revisions(
                &PhysicalGraphProjectionChange {
                    replacements,
                    deletions: source_delta.deletions,
                    reference_postings,
                },
                &replacement_sources,
            )
            .map_err(|error| error.to_string())?;
    }
    if !deltas.is_empty() {
        let mut replacements = Vec::new();
        let mut reference_postings = Vec::new();
        let mut replacement_sources = Vec::new();
        let mut deletions = Vec::new();
        for (_, (_, delta)) in deltas {
            match delta {
                PageDelta::Replace(entry, document, revision) => {
                    replacement_sources.push(PhysicalGraphProjectionSourceRevision {
                        page_id: page_id(&entry.rel_path),
                        revision,
                    });
                    let (page, mut postings) = physical_page(&entry, &document)?;
                    replacements.push(page);
                    reference_postings.append(&mut postings);
                }
                PageDelta::Delete(entry) => deletions.push(page_id(&entry.rel_path)),
            }
        }
        database
            .apply_with_source_revisions(
                &PhysicalGraphProjectionChange {
                    replacements,
                    deletions,
                    reference_postings,
                },
                &replacement_sources,
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn physical_page(
    entry: &PageEntry,
    document: &Document,
) -> Result<(PhysicalPage, Vec<PhysicalReferencePosting>), String> {
    #[cfg(test)]
    PHYSICAL_PAGE_LOWERINGS.fetch_add(1, Ordering::Relaxed);
    let id = page_id(&entry.rel_path);
    let is_org = Format::from_path(Path::new(&entry.rel_path)) == Format::Org;
    let (preamble_search, properties, tags) = document
        .pre_block
        .as_deref()
        .map(|raw| facets(raw, is_org))
        .unwrap_or_default();
    let searchable_text = if preamble_search.is_empty() {
        entry.name.clone()
    } else {
        format!("{} {preamble_search}", entry.name)
    };
    let mut blocks = Vec::new();
    let mut reference_postings = Vec::new();
    if let Some(preamble) = document.pre_block.as_deref() {
        append_reference_postings(
            &mut reference_postings,
            id,
            PhysicalEntityId::Page(id),
            b"preamble",
            std::iter::empty(),
            crate::doc::property_reference_page_names(preamble).into_iter(),
        )?;
    }
    lower_blocks(
        &document.roots,
        id,
        None,
        &mut Vec::new(),
        &mut blocks,
        &mut reference_postings,
    )?;
    Ok((
        PhysicalPage {
            page_id: id,
            home_document_id: id,
            name: entry.name.clone(),
            name_key: crate::refs::page_key(&entry.name),
            path: entry.rel_path.clone(),
            text_kind: page_kind_to_sql(entry.kind),
            preamble: document.pre_block.clone(),
            normalized_searchable_text: searchable_text.to_lowercase().nfc().collect(),
            searchable_text,
            references: Vec::new(),
            properties,
            tags,
            blocks,
        },
        reference_postings,
    ))
}

fn lower_blocks(
    source: &[DocBlock],
    page_id: [u8; 16],
    parent: Option<[u8; 16]>,
    structural_path: &mut Vec<u32>,
    out: &mut Vec<PhysicalBlock>,
    reference_postings: &mut Vec<PhysicalReferencePosting>,
) -> Result<(), String> {
    for (position, block) in source.iter().enumerate() {
        let position = u32::try_from(position)
            .map_err(|_| "page has more than u32::MAX sibling blocks".to_string())?;
        structural_path.push(position);
        let block_id = Uuid::parse_str(&block.uuid)
            .map_err(|_| {
                format!(
                    "block has no assigned runtime UUID in projection: {}",
                    block.uuid
                )
            })?
            .into_bytes();
        let projection = block.projection();
        let order = structural_path
            .iter()
            .map(|part| format!("{part:08x}"))
            .collect::<Vec<_>>()
            .join("/");
        append_reference_postings(
            reference_postings,
            page_id,
            PhysicalEntityId::Block(block_id),
            order.as_bytes(),
            projection.refs_page.iter().cloned(),
            crate::doc::property_reference_page_names(&block.raw).into_iter(),
        )?;
        let searchable_text = projection
            .visible
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let properties = projection
            .properties
            .iter()
            .map(|(name, value)| PhysicalProperty {
                name: name.clone(),
                normalized_name: property_key_norm(name),
                value: value.clone(),
            })
            .collect();
        out.push(PhysicalBlock {
            block_id,
            home_document_id: page_id,
            parent,
            order,
            content: block.raw.clone(),
            normalized_searchable_text: searchable_text.to_lowercase().nfc().collect(),
            searchable_text,
            heading_level: projection.heading_level,
            collapsed: block.collapsed(),
            logseq_uuid: None,
            logseq_identity_origin: None,
            references: Vec::new(),
            properties,
            tags: projection.tags.clone(),
            task: projection.marker.as_ref().map(|marker| PhysicalTask {
                marker: marker.to_ascii_uppercase(),
                priority: projection.priority.clone(),
                scheduled: projection.scheduled.clone(),
                deadline: projection.deadline.clone(),
            }),
        });
        lower_blocks(
            &block.children,
            page_id,
            Some(block_id),
            structural_path,
            out,
            reference_postings,
        )?;
        structural_path.pop();
    }
    Ok(())
}

fn append_reference_postings(
    out: &mut Vec<PhysicalReferencePosting>,
    page_id: [u8; 16],
    source: PhysicalEntityId,
    source_locator: &[u8],
    inline_names: impl IntoIterator<Item = String>,
    property_names: impl IntoIterator<Item = String>,
) -> Result<(), String> {
    let mut ordinal = 0_u32;
    for (kind, names) in [
        (0_i64, inline_names.into_iter().collect::<Vec<_>>()),
        (3_i64, property_names.into_iter().collect::<Vec<_>>()),
    ] {
        for raw_name in names {
            out.push(PhysicalReferencePosting {
                source_page_id: page_id,
                source_entity: source,
                source_locator: source_locator.to_vec(),
                ordinal,
                kind,
                target: PhysicalReferenceTarget::PageName {
                    normalized_name: crate::refs::page_key(&raw_name),
                    raw_name,
                    resolved_page_id: None,
                },
            });
            ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| "one reference source exceeds u32::MAX postings".to_string())?;
        }
    }
    Ok(())
}

fn facets(raw: &str, is_org: bool) -> (String, Vec<PhysicalProperty>, Vec<String>) {
    let mut block = DocBlock::new(raw);
    block.is_org = is_org;
    let searchable = block
        .visible_text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let properties = block
        .projection()
        .properties
        .iter()
        .map(|(name, value)| PhysicalProperty {
            name: name.clone(),
            normalized_name: property_key_norm(name),
            value: value.clone(),
        })
        .collect();
    (searchable, properties, block.projection().tags.clone())
}

pub(crate) fn page_id(relative_path: &str) -> [u8; 16] {
    let mut digest = Sha256::new();
    digest.update(b"tine-direct-page-v1\0");
    digest.update(relative_path.as_bytes());
    let bytes = digest.finalize();
    let mut id = [0; 16];
    id.copy_from_slice(&bytes[..16]);
    id
}

fn page_kind_to_sql(kind: PageKind) -> i64 {
    match kind {
        PageKind::Page => 0,
        PageKind::Journal => 1,
    }
}

fn page_kind_from_sql(kind: i64) -> Option<PageKind> {
    match kind {
        0 => Some(PageKind::Page),
        1 => Some(PageKind::Journal),
        _ => None,
    }
}

fn page_recency(root: &Path, name: &str, relative_path: &str, kind: i64) -> i64 {
    if kind == 1 {
        return JournalDate::from_title(name)
            .map(|date| date.to_days() * 86_400)
            .unwrap_or(i64::MIN);
    }
    std::fs::metadata(root.join(relative_path))
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(i64::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Graph;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    static PROJECTION_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tine-direct-projection-{tag}-{}", Uuid::new_v4()))
    }

    fn reset_lowerings() {
        PHYSICAL_PAGE_LOWERINGS.store(0, Ordering::Relaxed);
    }

    fn lowerings() -> u64 {
        PHYSICAL_PAGE_LOWERINGS.load(Ordering::Relaxed)
    }

    fn signature(groups: &[crate::model::RefGroup]) -> Vec<(String, Vec<(String, String)>)> {
        groups
            .iter()
            .map(|group| {
                (
                    group.page.clone(),
                    group
                        .blocks
                        .iter()
                        .map(|block| (block.id.clone(), block.raw.clone()))
                        .collect(),
                )
            })
            .collect()
    }

    fn wait_ready(graph: &Graph) {
        let started = Instant::now();
        while !graph.direct_projection_ready_test() {
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "Direct Files projection did not converge"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn direct_projection_matches_parser_tasks_and_tracks_replace_delete() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("task-parity");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(
            root.join("pages/tasks.md"),
            "- TODO [#A] parent\n\t- TODO child\n- TODO other\n  SCHEDULED: <2026-08-13 Thu>\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/org.org"), "* TODO [#B] org task\n").unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        for query in [
            "(task TODO)",
            "(and (task TODO) (priority A))",
            "(and (task TODO) (scheduled))",
            "(and (task TODO) (sort-by priority desc))",
        ] {
            let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
            let indexed = graph.run_query_bounded(query, 100, 1_000_000);
            assert_eq!(
                signature(&indexed.groups),
                signature(&oracle.groups),
                "{query}"
            );
            assert_eq!(
                (indexed.total, indexed.exceeded),
                (oracle.total, oracle.exceeded)
            );
        }
        assert!(graph.direct_projection_indexed_reads_test() >= 4);
        let indexed_reads = graph.direct_projection_indexed_reads_test();
        let repeated = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(
            signature(&repeated.groups),
            signature(
                &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
            )
        );
        assert_eq!(
            graph.direct_projection_indexed_reads_test(),
            indexed_reads,
            "the generation-keyed presentation memo must avoid repeated SQL/parser work"
        );

        let entry = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.name == "tasks")
            .unwrap();
        let mut page = graph.load_page(&entry).unwrap();
        let baseline = page.rev.clone();
        page.blocks[0].raw = "DONE [#A] parent".into();
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        for query in ["(task TODO)", "(task DONE)"] {
            let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
            let indexed = graph.run_query_bounded(query, 100, 1_000_000);
            assert_eq!(
                signature(&indexed.groups),
                signature(&oracle.groups),
                "{query}"
            );
        }

        graph.delete_page("org", PageKind::Page).unwrap();
        wait_ready(&graph);
        let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000);
        let indexed = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_projection_matches_fuzzy_search_and_virtual_reference_names() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("search-reference-parity");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/one.md"),
            "tags:: Page Tag, [[Property Page]]\nalias:: Alias Page\nquoted:: untouched\n\n- Characteristically useful [[Inline Page]]\n  aliases:: #Block Alias\n- c% literal\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/two.md"), "- unrelated content\n").unwrap();
        let graph = Graph::open(&root);
        graph.warm_cache();
        let oracle = crate::query::search(&graph, "cly", 20);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        wait_ready(&graph);

        let candidate_pages = graph
            .direct_projection_fuzzy_candidate_pages("cly")
            .unwrap();
        assert_eq!(candidate_pages.len(), 1);
        assert_eq!(candidate_pages[0].0.rel_path, "pages/one.md");
        assert_eq!(signature(&graph.search("cly", 20)), signature(&oracle));
        assert!(graph.direct_projection_fuzzy_candidate_reads_test() > 0);
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            names,
            [
                "page tag",
                "property page",
                "alias page",
                "inline page",
                "block",
                "block alias",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
        assert!(graph.direct_projection_referenced_name_reads_test() > 0);

        let fuzzy_reads = graph.direct_projection_fuzzy_candidate_reads_test();
        let name_reads = graph.direct_projection_referenced_name_reads_test();
        graph.direct_projection_mark_stale_test();
        assert_eq!(signature(&graph.search("cly", 20)), signature(&oracle));
        assert_eq!(
            graph
                .referenced_page_names()
                .into_iter()
                .map(|name| crate::refs::page_key(&name))
                .collect::<std::collections::BTreeSet<_>>(),
            names
        );
        assert_eq!(
            graph.direct_projection_fuzzy_candidate_reads_test(),
            fuzzy_reads,
            "a stale generation must use the parser fallback"
        );
        assert_eq!(
            graph.direct_projection_referenced_name_reads_test(),
            name_reads,
            "a stale generation must not read reference names from SQLite"
        );

        let entry = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.name == "one")
            .unwrap();
        let mut page = graph.load_page(&entry).unwrap();
        let baseline = page.rev.clone();
        page.blocks[0].raw = "Nothing matching [[Replacement Page]]".into();
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        assert!(graph.search("cly", 20).is_empty());
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("replacement page"));
        assert!(!names.contains("inline page"));

        std::fs::write(
            root.join("pages/one.md"),
            "tags:: External Tag\n\n- Externally changed fuzzy [[External Page]]\n",
        )
        .unwrap();
        graph.sync_file_checked(&root.join("pages/one.md")).unwrap();
        wait_ready(&graph);
        assert!(!graph.search("ecf", 20).is_empty());
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("external tag"));
        assert!(names.contains("external page"));
        assert!(!names.contains("replacement page"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_projection_fuzzy_candidates_preserve_parser_corpus_semantics() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("search-corpus-parity");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/search.md"),
            "- Characteristically useful\n  - descendant Needle\n- Café and cafe\u{301}\n- 100% under_score back\\slash\n- MixedCASE\n- x a y b z\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pages/other.md"),
            "- Another characteristically useful result\n",
        )
        .unwrap();
        let cases = [
            ("", 20),
            ("   ", 20),
            ("cly", 20),
            ("needle", 20),
            ("CAFÉ", 20),
            ("cafe\u{301}", 20),
            ("%", 20),
            ("_", 20),
            ("\\", 20),
            ("mixedcase", 20),
            ("xyz", 20),
            ("cly", 1),
        ];
        let oracle_graph = Graph::open(&root);
        oracle_graph.warm_cache();
        let oracle = cases
            .iter()
            .map(|(query, limit)| signature(&crate::query::search(&oracle_graph, query, *limit)))
            .collect::<Vec<_>>();
        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        assert!(
            graph.warm_cache_cancellable(|| false),
            "corpus cache failed to warm: {:?}",
            graph.page_index_failures()
        );
        wait_ready(&graph);
        for ((query, limit), expected) in cases.into_iter().zip(oracle) {
            assert_eq!(
                signature(&graph.search(query, limit)),
                expected,
                "{query:?}"
            );
        }
        let cancellation_checks = std::cell::Cell::new(0);
        assert!(crate::query::search_cancellable(&graph, "cly", 20, || {
            cancellation_checks.set(cancellation_checks.get() + 1);
            cancellation_checks.get() > 1
        })
        .is_empty());

        graph.rename_page("search", "renamed search").unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(
            signature(&graph.search("needle", 20)),
            signature(&crate::query::search(&graph, "needle", 20))
        );
        graph.delete_page("renamed search", PageKind::Page).unwrap();
        wait_ready(&graph);
        assert!(graph.search("needle", 20).is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unavailable_projection_keeps_direct_files_query_semantics() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("fallback");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/tasks.md"),
            "- TODO Characteristically readable [[Inline Only]]\n  alias:: #Alias Only\n",
        )
        .unwrap();
        let blocked_parent = root.join("not-a-directory");
        std::fs::write(&blocked_parent, b"ordinary file").unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(blocked_parent.join("projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        std::thread::sleep(Duration::from_millis(30));
        let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000);
        let fallback = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(signature(&fallback.groups), signature(&oracle.groups));
        assert_eq!(graph.direct_projection_indexed_reads_test(), 0);
        assert_eq!(
            signature(&graph.search("cly", 20)),
            signature(&crate::query::search(&graph, "cly", 20))
        );
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("inline only"));
        assert!(names.contains("alias only"));
        assert_eq!(graph.direct_projection_fuzzy_candidate_reads_test(), 0);
        assert_eq!(graph.direct_projection_referenced_name_reads_test(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_graph_instance_cannot_replace_ready_projection_facts() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("single-writer");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/tasks.md"), "- TODO one\n").unwrap();
        let database = scratch("single-writer-db").join("projection.sqlite");

        let owner = Graph::open(&root);
        owner.attach_direct_projection(database.clone()).unwrap();
        owner.warm_cache();
        wait_ready(&owner);

        let fallback = Graph::open(&root);
        fallback.attach_direct_projection(database.clone()).unwrap();
        fallback.warm_cache();
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !fallback.direct_projection_ready_test(),
            "a second graph instance must not publish into the first instance's ready database"
        );
        let oracle = crate::query::run_query_bounded(&fallback, "(task TODO)", 100, 1_000_000);
        let actual = fallback.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(signature(&actual.groups), signature(&oracle.groups));
        assert_eq!(fallback.direct_projection_indexed_reads_test(), 0);

        let owner_oracle = crate::query::run_query_bounded(&owner, "(task TODO)", 100, 1_000_000);
        let owner_actual = owner.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(
            signature(&owner_actual.groups),
            signature(&owner_oracle.groups)
        );
        assert!(owner.direct_projection_indexed_reads_test() > 0);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }

    #[test]
    fn clean_reopen_reuses_sqlite_and_external_edit_relowers_only_one_page() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("reopen-revisions");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
        std::fs::write(root.join("pages/two.md"), "- DONE two\n").unwrap();
        let database = scratch("reopen-revisions-db").join("projection.sqlite");

        reset_lowerings();
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            assert_eq!(lowerings(), 2);
        }
        std::thread::sleep(Duration::from_millis(20));

        reset_lowerings();
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            assert_eq!(lowerings(), 0, "unchanged pages must stay inside SQLite");
        }
        std::thread::sleep(Duration::from_millis(20));

        std::fs::write(root.join("pages/one.md"), "- TODO one changed\n").unwrap();
        reset_lowerings();
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            assert_eq!(
                lowerings(),
                1,
                "one changed page must produce one SQL delta"
            );
            assert_eq!(
                signature(
                    &graph
                        .run_query_bounded("(task TODO)", 100, 1_000_000)
                        .groups
                ),
                signature(
                    &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
                )
            );
        }

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }

    #[test]
    fn storage_contract_names_the_generation_bound_cutover() {
        let contract = include_str!("../../../docs/storage-sync-contract.md");
        assert!(contract.contains("direct-files-projections/<canonical-graph-path-digest>.sqlite"));
        assert!(contract.contains("sparse_task_query_eligibility"));
        assert!(contract.contains("literal fuzzy-search candidate"));
        assert!(contract.contains("referenced-page\ninventory"));
        assert!(contract.contains("retains no separate semantic memo"));
        assert!(contract.contains("exact current parser-cache\ngeneration"));
        assert!(contract.contains("app-private graph-fact projection contains no managed state"));
        assert!(contract.contains("clean\nreopen lowers none"));
        assert!(
            contract.contains("memo of already-shaped frontend result DTOs remains Tine-native")
        );
        assert!(contract.contains("grants no\n   authority"));
    }

    #[test]
    #[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
    fn real_corpus_projection_converges_and_matches_task_query() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = PathBuf::from(
            std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
                .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
        );
        let database = scratch("real-corpus").join("projection.sqlite");
        let oracle_graph = Graph::open(&root);
        oracle_graph.warm_cache();
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        let started = Instant::now();
        graph.warm_cache();
        let warm = started.elapsed();
        wait_ready(&graph);
        let converged = started.elapsed();
        let oracle_started = Instant::now();
        let oracle =
            crate::query::run_query_bounded(&oracle_graph, "(task TODO)", 20_000, 32 << 20);
        let oracle_elapsed = oracle_started.elapsed();
        let query_started = Instant::now();
        let indexed = graph.run_query_bounded("(task TODO)", 20_000, 32 << 20);
        let indexed_elapsed = query_started.elapsed();
        assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
        let indexed_reads = graph.direct_projection_indexed_reads_test();
        let memo_started = Instant::now();
        let repeated = graph.run_query_bounded("(task TODO)", 20_000, 32 << 20);
        let memo_elapsed = memo_started.elapsed();
        assert_eq!(signature(&repeated.groups), signature(&oracle.groups));
        assert_eq!(graph.direct_projection_indexed_reads_test(), indexed_reads);
        let mut fuzzy_indexed = Duration::ZERO;
        let mut fuzzy_oracle = Duration::ZERO;
        for value in ["a", "todo", "http", "2026", "%", "_", "é"] {
            let indexed_started = Instant::now();
            let indexed_search = graph.search(value, 5_000);
            fuzzy_indexed += indexed_started.elapsed();
            let oracle_started = Instant::now();
            let oracle_search = crate::query::search(&oracle_graph, value, 5_000);
            fuzzy_oracle += oracle_started.elapsed();
            assert_eq!(
                signature(&indexed_search),
                signature(&oracle_search),
                "real-corpus fuzzy search diverged for a bounded probe"
            );
        }
        eprintln!(
            "direct projection fuzzy receipt: indexed_total_ms={} oracle_total_ms={}",
            fuzzy_indexed.as_millis(),
            fuzzy_oracle.as_millis(),
        );
        let normalize_names = |mut names: Vec<String>| {
            names.sort_by_key(|name| crate::refs::page_key(name));
            names
        };
        assert_eq!(
            normalize_names(graph.referenced_page_names()),
            normalize_names(oracle_graph.referenced_page_names()),
            "real-corpus referenced-page inventory diverged"
        );
        assert!(graph.direct_projection_fuzzy_candidate_reads_test() > 0);
        assert!(graph.direct_projection_referenced_name_reads_test() > 0);
        let task_candidates = PhysicalGraphProjectionDatabase::open_read_only(&database)
            .unwrap()
            .read()
            .task_candidate_blocks_after("TODO", None, 10_000)
            .unwrap()
            .len();
        eprintln!(
            "direct projection receipt: warm_ms={} projection_total_ms={} oracle_query_us={} indexed_query_us={} repeated_query_us={} pages={} task_candidates={}",
            warm.as_millis(),
            converged.as_millis(),
            oracle_elapsed.as_micros(),
            indexed_elapsed.as_micros(),
            memo_elapsed.as_micros(),
            graph.list_pages().len(),
            task_candidates,
        );
    }

    #[test]
    #[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
    fn real_corpus_clean_reopen_reuses_projected_pages() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = PathBuf::from(
            std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
                .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
        );
        let database = scratch("real-corpus-reopen").join("projection.sqlite");
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
        }
        std::thread::sleep(Duration::from_millis(20));

        reset_lowerings();
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        let started = Instant::now();
        graph.warm_cache();
        let warm = started.elapsed();
        wait_ready(&graph);
        let converged = started.elapsed();
        let query_started = Instant::now();
        let indexed = graph.run_query_bounded("(task TODO)", 20_000, 32 << 20);
        let indexed_elapsed = query_started.elapsed();
        let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 20_000, 32 << 20);
        assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
        assert_eq!(
            lowerings(),
            0,
            "clean reopen must not lower unchanged pages"
        );
        eprintln!(
            "direct projection clean-reopen receipt: warm_ms={} projection_total_ms={} projection_tail_ms={} indexed_query_us={} pages_lowered={}",
            warm.as_millis(),
            converged.as_millis(),
            converged.saturating_sub(warm).as_millis(),
            indexed_elapsed.as_micros(),
            lowerings(),
        );
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }
}
