use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use loro::{
    Container, ExportMode, LoroDoc, LoroMap, LoroText, LoroTree, LoroValue, TreeID, TreeParentId,
    UpdateOptions, ValueOrContainer, VersionVector,
};
use uuid::Uuid;

use super::snapshot::validate_pages;
use super::store::{chunk_ids, Chunk, ChunkKind, Store};
use super::{
    AffectedPage, BlockId, BlockSnapshot, CommitReport, CrdtError, CrdtStatus, ImportReport,
    PageId, PageSelector, PageSnapshot,
};

const GRAPH_TREE: &str = "graph";
const NODE_TYPE: &str = "node_type";
const PAGE_NODE: &str = "page";
const BLOCK_NODE: &str = "block";
const PAGE_ID: &str = "page_id";
const BLOCK_ID: &str = "block_id";
const PATH: &str = "path";
const NAME: &str = "name";
const KIND: &str = "kind";
const FORMAT: &str = "format";
const RAW: &str = "raw";
const PRE_BLOCK: &str = "pre_block";
const PRE_BLOCK_PRESENT: &str = "pre_block_present";

/// A standalone, graph-wide Loro document backed by immutable managed-sync
/// chunks under `.tine-sync/v1`.
#[derive(Debug)]
pub struct CrdtGraph {
    doc: LoroDoc,
    store: Store,
    imported_chunks: HashSet<String>,
    durability_blocked: Option<String>,
}

impl CrdtGraph {
    /// Creates a new workspace and its single genesis chunk.
    ///
    /// `session_id` must be freshly generated for every process. Reusing it is
    /// rejected because Loro peers must never have concurrent writers.
    pub fn initialize(
        sync_root: impl AsRef<Path>,
        device_id: Uuid,
        session_id: Uuid,
        pages: Vec<PageSnapshot>,
    ) -> Result<Self, CrdtError> {
        validate_pages(&pages)?;

        let doc = new_doc(session_id)?;
        apply_pages(&doc, &pages)?;
        let payload = doc.export(ExportMode::all_updates()).map_err(loro_error)?;

        let store = Store::initialize(sync_root.as_ref(), device_id, session_id)?;
        let affected_pages = pages
            .iter()
            .map(|page| AffectedPage {
                page_id: page.id,
                paths: vec![page.path.clone()],
            })
            .collect();
        let chunk_id = store.publish(ChunkKind::Genesis, affected_pages, payload)?;

        Ok(Self {
            doc,
            store,
            imported_chunks: HashSet::from([chunk_id]),
            durability_blocked: None,
        })
    }

    /// Opens an existing workspace by replaying all immutable chunks.
    pub fn open(
        sync_root: impl AsRef<Path>,
        device_id: Uuid,
        session_id: Uuid,
    ) -> Result<Self, CrdtError> {
        let (store, chunks) = Store::open(sync_root.as_ref(), device_id, session_id)?;
        let doc = replay_chunks(&chunks, session_id)?;
        // Validate the application schema before exposing the document.
        materialize_pages_from(&doc)?;
        Ok(Self {
            doc,
            store,
            imported_chunks: chunk_ids(&chunks),
            durability_blocked: None,
        })
    }

    /// Reconciles and durably records one complete page snapshot.
    ///
    /// Requested block IDs are resolved globally. This permits a destination
    /// page to claim an existing block before the source page projection is
    /// committed, while preserving the block's Loro tree identity and text.
    pub fn commit_page(&mut self, snapshot: PageSnapshot) -> Result<CommitReport, CrdtError> {
        self.commit_pages(vec![snapshot])
    }

    /// Reconciles several complete page snapshots as one Loro transaction and
    /// one immutable update chunk. Used by transactional graph operations such as
    /// namespace rename, where path changes and reference rewrites must not be
    /// observed as separate sync commits.
    pub fn commit_pages(
        &mut self,
        snapshots: Vec<PageSnapshot>,
    ) -> Result<CommitReport, CrdtError> {
        self.ensure_writable()?;
        validate_pages(&snapshots)?;
        for snapshot in &snapshots {
            validate_commit_against_doc(&self.doc, snapshot)?;
        }
        let mut affected_pages = Vec::with_capacity(snapshots.len());
        for snapshot in &snapshots {
            let mut paths = BTreeSet::from([snapshot.path.clone()]);
            if let Some(old_path) = find_page_node(&self.doc, PageSelector::Id(snapshot.id))?
                .map(|node| required_string(&tree(&self.doc), node, PATH))
                .transpose()?
            {
                paths.insert(old_path);
            }
            affected_pages.push(AffectedPage {
                page_id: snapshot.id,
                paths: paths.into_iter().collect(),
            });
        }
        let before = self.doc.oplog_vv();
        if let Err(error) = apply_pages(&self.doc, &snapshots) {
            self.recover_after_failed_mutation(&error);
            return Err(error);
        }
        let affected_page_ids: Vec<PageId> =
            affected_pages.iter().map(|page| page.page_id).collect();
        let affected_paths: Vec<String> = affected_pages
            .iter()
            .flat_map(|page| page.paths.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if self.doc.oplog_vv() == before {
            return Ok(CommitReport {
                changed: false,
                chunk_id: String::new(),
                affected_page_ids,
                affected_paths,
                affected_pages,
            });
        }
        self.persist_update(before, affected_pages)
    }

    /// Replaces the complete graph state as one operation. This is used for a
    /// verified backup restore: the operation becomes durable before projection
    /// files are copied, so a crash can resume the projection instead of silently
    /// reverting the restore on the next replay.
    pub fn replace_pages(
        &mut self,
        snapshots: Vec<PageSnapshot>,
    ) -> Result<CommitReport, CrdtError> {
        self.ensure_writable()?;
        validate_pages(&snapshots)?;
        for snapshot in &snapshots {
            validate_commit_against_doc(&self.doc, snapshot)?;
        }

        let current = materialize_pages_from(&self.doc)?;
        let mut affected: HashMap<PageId, BTreeSet<String>> = HashMap::new();
        for page in current.iter().chain(snapshots.iter()) {
            affected
                .entry(page.id)
                .or_default()
                .insert(page.path.clone());
        }
        let before = self.doc.oplog_vv();
        if let Err(error) = apply_pages(&self.doc, &snapshots) {
            self.recover_after_failed_mutation(&error);
            return Err(error);
        }

        let target_ids: HashSet<PageId> = snapshots.iter().map(|page| page.id).collect();
        let tree = tree(&self.doc);
        for page in current {
            if target_ids.contains(&page.id) {
                continue;
            }
            if let Some(node) = find_page_node(&self.doc, PageSelector::Id(page.id))? {
                if let Err(error) = tree.delete(node).map_err(loro_error) {
                    self.recover_after_failed_mutation(&error);
                    return Err(error);
                }
            }
        }
        if let Err(error) = materialize_pages_from(&self.doc) {
            self.recover_after_failed_mutation(&error);
            return Err(error);
        }

        let mut affected_pages: Vec<AffectedPage> = affected
            .into_iter()
            .map(|(page_id, paths)| AffectedPage {
                page_id,
                paths: paths.into_iter().collect(),
            })
            .collect();
        affected_pages.sort_by_key(|page| page.page_id);
        if self.doc.oplog_vv() == before {
            let affected_page_ids = affected_pages.iter().map(|page| page.page_id).collect();
            let affected_paths = affected_pages
                .iter()
                .flat_map(|page| page.paths.iter().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            return Ok(CommitReport {
                changed: false,
                chunk_id: String::new(),
                affected_page_ids,
                affected_paths,
                affected_pages,
            });
        }
        let authorized_paths = affected_pages
            .iter()
            .flat_map(|page| page.paths.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.persist_update_with_intents(before, affected_pages, authorized_paths)
    }

    /// Deletes a page and its currently attached block subtree by ID or path.
    pub fn delete_page(
        &mut self,
        page: impl Into<PageSelector>,
    ) -> Result<CommitReport, CrdtError> {
        self.ensure_writable()?;
        let page = page.into();
        let node = find_page_node(&self.doc, page)?.ok_or(CrdtError::PageNotFound)?;
        let tree = tree(&self.doc);
        let id = PageId(parse_uuid(
            &required_string(&tree, node, PAGE_ID)?,
            PAGE_ID,
        )?);
        let path = required_string(&tree, node, PATH)?;
        let before = self.doc.oplog_vv();
        if let Err(error) = tree.delete(node).map_err(loro_error) {
            self.recover_after_failed_mutation(&error);
            return Err(error);
        }
        self.persist_update(
            before,
            vec![AffectedPage {
                page_id: id,
                paths: vec![path],
            }],
        )
    }

    /// Imports all newly delivered chunks and reports pages that need projection.
    /// Importing into a fork keeps malformed or incomplete input from partially
    /// changing live state without replaying all historical payloads each time.
    pub fn import_pending(&mut self) -> Result<ImportReport, CrdtError> {
        let new_chunks = self.store.load_new_chunks(&self.imported_chunks)?;
        if new_chunks.is_empty() {
            return Ok(ImportReport::default());
        }

        let candidate = self.doc.fork();
        candidate
            .set_peer_id(peer_id(self.store.session_id))
            .map_err(loro_error)?;
        let payloads: Vec<Vec<u8>> = new_chunks
            .iter()
            .map(|chunk| chunk.payload.clone())
            .collect();
        let status = candidate.import_batch(&payloads).map_err(loro_error)?;
        if status.pending.is_some() {
            return Err(CrdtError::InvalidChunk(
                "new chunks have unresolved Loro dependencies".into(),
            ));
        }
        materialize_pages_from(&candidate)?;

        let mut affected: HashMap<PageId, BTreeSet<String>> = HashMap::new();
        for chunk in &new_chunks {
            for page in &chunk.header.affected_pages {
                affected
                    .entry(page.page_id)
                    .or_default()
                    .extend(page.paths.iter().cloned());
            }
        }
        let mut affected_pages: Vec<AffectedPage> = affected
            .into_iter()
            .map(|(page_id, paths)| AffectedPage {
                page_id,
                paths: paths.into_iter().collect(),
            })
            .collect();
        affected_pages.sort_by_key(|page| page.page_id);
        let page_ids = affected_pages.iter().map(|page| page.page_id).collect();
        let paths = affected_pages
            .iter()
            .flat_map(|page| page.paths.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let report = ImportReport {
            imported_chunks: new_chunks.len(),
            affected_page_ids: page_ids,
            affected_paths: paths,
            affected_pages,
        };
        self.doc = candidate;
        self.imported_chunks
            .extend(new_chunks.into_iter().map(|chunk| chunk.id));
        Ok(report)
    }

    /// Materializes one current page projection.
    pub fn materialize_page(
        &self,
        page: impl Into<PageSelector>,
    ) -> Result<Option<PageSnapshot>, CrdtError> {
        let selector = page.into();
        Ok(materialize_pages_from(&self.doc)?
            .into_iter()
            .find(|snapshot| match &selector {
                PageSelector::Id(id) => snapshot.id == *id,
                PageSelector::Path(path) => snapshot.path == *path,
            }))
    }

    /// Materializes every page, sorted by path and immutable page ID.
    pub fn materialize_pages(&self) -> Result<Vec<PageSnapshot>, CrdtError> {
        materialize_pages_from(&self.doc)
    }

    /// Every page/path pair ever named by an immutable chunk. Current snapshots
    /// alone cannot reveal a deleted page or the old half of a rename after a
    /// restart, but projection recovery still needs those paths.
    pub fn affected_pages_history(&self) -> Result<Vec<AffectedPage>, CrdtError> {
        let mut affected: HashMap<PageId, BTreeSet<String>> = HashMap::new();
        for chunk in self.store.load_chunks()? {
            for page in chunk.header.affected_pages {
                affected.entry(page.page_id).or_default().extend(page.paths);
            }
        }
        let mut pages: Vec<AffectedPage> = affected
            .into_iter()
            .map(|(page_id, paths)| AffectedPage {
                page_id,
                paths: paths.into_iter().collect(),
            })
            .collect();
        pages.sort_by_key(|page| page.page_id);
        Ok(pages)
    }

    /// Returns managed-sync identity and local import state.
    pub fn status(&self) -> Result<CrdtStatus, CrdtError> {
        Ok(CrdtStatus {
            workspace_id: self.store.workspace_id,
            device_id: self.store.device_id,
            session_id: self.store.session_id,
            page_count: self.materialize_pages()?.len(),
            imported_chunks: self.imported_chunks.len(),
            store_root: self.store.root.clone(),
            durability_blocked: self.durability_blocked.is_some(),
        })
    }

    /// Publish immutable provenance for an exact Markdown/Org projection at the
    /// document's current operation frontier.
    pub fn record_projection(&self, path: &str, content: &str) -> Result<String, CrdtError> {
        self.store
            .publish_projection_receipt(path, content, self.doc.oplog_vv())
    }

    /// Whether `content` at `path` was projected by a replica from a frontier
    /// already included in this document. A receipt delivered ahead of its update
    /// chunks therefore does not authorize an overwrite or automatic cleanup.
    pub fn is_known_projection(&self, path: &str, content: &str) -> Result<bool, CrdtError> {
        self.store
            .is_known_projection(path, content, &self.doc.oplog_vv())
    }

    pub fn is_projection_authorized(&self, path: &str) -> Result<bool, CrdtError> {
        self.store
            .is_projection_authorized(path, &self.doc.oplog_vv())
    }

    fn ensure_writable(&self) -> Result<(), CrdtError> {
        match &self.durability_blocked {
            Some(error) => Err(CrdtError::DurabilityBlocked(error.clone())),
            None => Ok(()),
        }
    }

    fn persist_update(
        &mut self,
        before: VersionVector,
        affected_pages: Vec<AffectedPage>,
    ) -> Result<CommitReport, CrdtError> {
        self.persist_update_with_intents(before, affected_pages, Vec::new())
    }

    fn persist_update_with_intents(
        &mut self,
        before: VersionVector,
        affected_pages: Vec<AffectedPage>,
        authorized_paths: Vec<String>,
    ) -> Result<CommitReport, CrdtError> {
        let payload = match self.doc.export(ExportMode::updates(&before)) {
            Ok(payload) => payload,
            Err(error) => {
                let error = loro_error(error);
                self.recover_after_failed_mutation(&error);
                return Err(error);
            }
        };
        let frontier = self.doc.oplog_vv();
        for path in authorized_paths {
            if let Err(error) = self
                .store
                .publish_projection_intent(&path, frontier.clone())
            {
                self.recover_after_failed_mutation(&error);
                return Err(error);
            }
        }
        match self
            .store
            .publish(ChunkKind::Update, affected_pages.clone(), payload)
        {
            Ok(chunk_id) => {
                self.imported_chunks.insert(chunk_id.clone());
                let affected_page_ids = affected_pages.iter().map(|page| page.page_id).collect();
                let affected_paths = affected_pages
                    .iter()
                    .flat_map(|page| page.paths.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                Ok(CommitReport {
                    changed: true,
                    chunk_id,
                    affected_page_ids,
                    affected_paths,
                    affected_pages,
                })
            }
            Err(error) => {
                self.recover_after_failed_mutation(&error);
                Err(error)
            }
        }
    }

    fn recover_after_failed_mutation(&mut self, original: &CrdtError) {
        match self.store.load_chunks().and_then(|chunks| {
            replay_chunks(&chunks, self.store.session_id).map(|doc| (doc, chunks))
        }) {
            Ok((doc, chunks)) => {
                self.doc = doc;
                self.imported_chunks = chunk_ids(&chunks);
            }
            Err(recovery) => {
                self.durability_blocked = Some(format!(
                    "original failure: {original}; persisted-state rebuild failed: {recovery}"
                ));
            }
        }
    }
}

fn new_doc(session_id: Uuid) -> Result<LoroDoc, CrdtError> {
    let doc = LoroDoc::new();
    doc.set_peer_id(peer_id(session_id)).map_err(loro_error)?;
    tree(&doc).enable_fractional_index(0);
    Ok(doc)
}

fn replay_chunks(chunks: &[Chunk], session_id: Uuid) -> Result<LoroDoc, CrdtError> {
    let doc = LoroDoc::new();
    let mut ordered: Vec<&Chunk> = chunks.iter().collect();
    ordered.sort_by_key(|chunk| match chunk.header.kind {
        ChunkKind::Genesis => 0,
        ChunkKind::Update => 1,
    });

    for chunk in &ordered {
        doc.import(&chunk.payload).map_err(loro_error)?;
    }
    // A second idempotent pass verifies that every out-of-order dependency is
    // now satisfiable. Loro retains pending updates between imports.
    for chunk in &ordered {
        let status = doc.import(&chunk.payload).map_err(loro_error)?;
        if status.pending.is_some() {
            return Err(CrdtError::InvalidChunk(format!(
                "chunk {} has unresolved Loro dependencies",
                chunk.id
            )));
        }
    }
    doc.set_peer_id(peer_id(session_id)).map_err(loro_error)?;
    tree(&doc).enable_fractional_index(0);
    Ok(doc)
}

fn peer_id(session_id: Uuid) -> u64 {
    let bytes = session_id.as_bytes();
    let mut peer = u64::from_be_bytes(bytes[..8].try_into().expect("UUID half is eight bytes"));
    if peer == u64::MAX {
        peer -= 1;
    }
    peer
}

fn tree(doc: &LoroDoc) -> LoroTree {
    doc.get_tree(GRAPH_TREE)
}

fn validate_commit_against_doc(doc: &LoroDoc, page: &PageSnapshot) -> Result<(), CrdtError> {
    for existing in materialize_pages_from(doc)? {
        if existing.id != page.id && existing.path == page.path {
            return Err(CrdtError::DuplicatePagePath(page.path.clone()));
        }
    }
    Ok(())
}

/// Apply a set of complete page snapshots without making block moves depend on
/// snapshot order. Cleanup happens only after every destination has claimed its
/// blocks, so swaps and cycles preserve the existing Loro nodes and identities.
fn apply_pages(doc: &LoroDoc, pages: &[PageSnapshot]) -> Result<(), CrdtError> {
    let tree = tree(doc);
    let mut original_nodes = HashSet::new();
    for page in pages {
        if let Some(node) = find_page_node(doc, PageSelector::Id(page.id))? {
            original_nodes.extend(descendants(&tree, node));
        }
    }
    for page in pages {
        apply_page(doc, page)?;
    }

    let desired_ids: HashSet<BlockId> = pages
        .iter()
        .flat_map(|page| page.blocks.iter().map(|block| block.id))
        .collect();
    let nodes_by_id = block_nodes_by_id(&tree)?;
    let desired_nodes: HashSet<TreeID> = desired_ids
        .iter()
        .filter_map(|id| nodes_by_id.get(id).copied())
        .collect();
    let removed: HashSet<TreeID> = original_nodes
        .into_iter()
        .filter(|node| !desired_nodes.contains(node))
        .collect();
    for node in removed.iter().copied().filter(|node| {
        !matches!(tree.parent(*node), Some(TreeParentId::Node(parent)) if removed.contains(&parent))
    }) {
        tree.delete(node).map_err(loro_error)?;
    }
    Ok(())
}

fn apply_page(doc: &LoroDoc, page: &PageSnapshot) -> Result<(), CrdtError> {
    let tree = tree(doc);
    let existing_page = find_page_node(doc, PageSelector::Id(page.id))?;
    let page_node = match existing_page {
        Some(node) => node,
        None => tree.create(None).map_err(loro_error)?,
    };
    if tree.parent(page_node) != Some(TreeParentId::Root) {
        tree.mov(page_node, None).map_err(loro_error)?;
    }

    let page_meta = tree.get_meta(page_node).map_err(loro_error)?;
    put_string(&page_meta, NODE_TYPE, PAGE_NODE)?;
    if existing_page.is_none() {
        put_string(&page_meta, PAGE_ID, &page.id.to_string())?;
    } else if required_string(&tree, page_node, PAGE_ID)? != page.id.to_string() {
        return Err(CrdtError::InvalidDocument(
            "attempted to change an immutable page UUID".into(),
        ));
    }
    put_string(&page_meta, PATH, &page.path)?;
    put_string(&page_meta, NAME, &page.name)?;
    put_string(&page_meta, KIND, &page.kind)?;
    put_string(&page_meta, FORMAT, &page.format)?;
    let pre_block = page_meta
        .ensure_mergeable_text(PRE_BLOCK)
        .map_err(loro_error)?;
    put_bool(&page_meta, PRE_BLOCK_PRESENT, page.pre_block.is_some())?;
    if let Some(value) = &page.pre_block {
        replace_text(&pre_block, value)?;
    }

    let global_blocks = block_nodes_by_id(&tree)?;
    let mut nodes = HashMap::with_capacity(page.blocks.len());
    for block in &page.blocks {
        let node = match global_blocks.get(&block.id) {
            Some(node) => *node,
            None => tree.create(page_node).map_err(loro_error)?,
        };
        let meta = tree.get_meta(node).map_err(loro_error)?;
        put_string(&meta, NODE_TYPE, BLOCK_NODE)?;
        if global_blocks.contains_key(&block.id) {
            if required_string(&tree, node, BLOCK_ID)? != block.id.to_string() {
                return Err(CrdtError::InvalidDocument(
                    "attempted to change an immutable block UUID".into(),
                ));
            }
        } else {
            put_string(&meta, BLOCK_ID, &block.id.to_string())?;
        }
        replace_text(
            &meta.ensure_mergeable_text(RAW).map_err(loro_error)?,
            &block.raw,
        )?;
        nodes.insert(block.id, node);
    }

    let mut depth_cache = HashMap::new();
    let block_by_id: HashMap<BlockId, &BlockSnapshot> =
        page.blocks.iter().map(|block| (block.id, block)).collect();
    let mut ordered: Vec<&BlockSnapshot> = page.blocks.iter().collect();
    ordered.sort_by_key(|block| {
        (
            block_depth(block.id, &block_by_id, &mut depth_cache),
            block.parent,
            block.order,
            block.id,
        )
    });
    for block in ordered {
        let parent = block
            .parent
            .and_then(|id| nodes.get(&id).copied())
            .unwrap_or(page_node);
        let node = nodes[&block.id];
        let current_parent = tree.parent(node);
        let requested_parent = TreeParentId::Node(parent);
        let current_index = tree
            .children(parent)
            .and_then(|children| children.iter().position(|child| *child == node));
        if current_parent != Some(requested_parent) || current_index != Some(block.order as usize) {
            tree.mov_to(node, parent, block.order as usize)
                .map_err(loro_error)?;
        }
    }

    Ok(())
}

fn block_depth(
    id: BlockId,
    blocks: &HashMap<BlockId, &BlockSnapshot>,
    cache: &mut HashMap<BlockId, usize>,
) -> usize {
    if let Some(depth) = cache.get(&id) {
        return *depth;
    }
    let depth = blocks[&id]
        .parent
        .map(|parent| block_depth(parent, blocks, cache) + 1)
        .unwrap_or(0);
    cache.insert(id, depth);
    depth
}

fn block_nodes_by_id(tree: &LoroTree) -> Result<HashMap<BlockId, TreeID>, CrdtError> {
    let mut blocks = HashMap::new();
    for node in tree.get_nodes(false) {
        let meta = tree.get_meta(node.id).map_err(loro_error)?;
        if optional_string(&meta, NODE_TYPE)?.as_deref() == Some(BLOCK_NODE) {
            let id = BlockId(parse_uuid(
                &required_map_string(&meta, BLOCK_ID)?,
                BLOCK_ID,
            )?);
            if blocks.insert(id, node.id).is_some() {
                return Err(CrdtError::DuplicateBlockId(id));
            }
        }
    }
    Ok(blocks)
}

fn find_page_node(doc: &LoroDoc, selector: PageSelector) -> Result<Option<TreeID>, CrdtError> {
    let tree = tree(doc);
    let mut found = None;
    for node in tree.roots() {
        let meta = tree.get_meta(node).map_err(loro_error)?;
        if optional_string(&meta, NODE_TYPE)?.as_deref() != Some(PAGE_NODE) {
            return Err(CrdtError::InvalidDocument(
                "root movable-tree node is not a page".into(),
            ));
        }
        let matches = match &selector {
            PageSelector::Id(id) => required_map_string(&meta, PAGE_ID)? == id.to_string(),
            PageSelector::Path(path) => required_map_string(&meta, PATH)? == *path,
        };
        if matches {
            if found.replace(node).is_some() {
                return Err(CrdtError::InvalidDocument(
                    "page selector matches multiple tree nodes".into(),
                ));
            }
        }
    }
    Ok(found)
}

fn materialize_pages_from(doc: &LoroDoc) -> Result<Vec<PageSnapshot>, CrdtError> {
    let tree = tree(doc);
    let mut pages = Vec::new();
    let mut block_ids = HashSet::new();
    for page_node in tree.roots() {
        let meta = tree.get_meta(page_node).map_err(loro_error)?;
        if required_map_string(&meta, NODE_TYPE)? != PAGE_NODE {
            return Err(CrdtError::InvalidDocument(
                "root movable-tree node is not a page".into(),
            ));
        }
        let id = PageId(parse_uuid(&required_map_string(&meta, PAGE_ID)?, PAGE_ID)?);
        let mut blocks = Vec::new();
        materialize_children(&tree, page_node, None, &mut blocks, &mut block_ids)?;
        pages.push(PageSnapshot {
            id,
            path: required_map_string(&meta, PATH)?,
            name: required_map_string(&meta, NAME)?,
            kind: required_map_string(&meta, KIND)?,
            format: required_map_string(&meta, FORMAT)?,
            pre_block: if required_bool(&meta, PRE_BLOCK_PRESENT)? {
                Some(required_text(&meta, PRE_BLOCK)?)
            } else {
                None
            },
            blocks,
        });
    }
    validate_pages(&pages)?;
    pages.sort_by(|left, right| left.path.cmp(&right.path).then(left.id.cmp(&right.id)));
    Ok(pages)
}

fn materialize_children(
    tree: &LoroTree,
    parent_node: TreeID,
    parent_id: Option<BlockId>,
    output: &mut Vec<BlockSnapshot>,
    global_ids: &mut HashSet<BlockId>,
) -> Result<(), CrdtError> {
    for (order, node) in tree
        .children(parent_node)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let meta = tree.get_meta(node).map_err(loro_error)?;
        if required_map_string(&meta, NODE_TYPE)? != BLOCK_NODE {
            return Err(CrdtError::InvalidDocument(
                "non-block node found below a page".into(),
            ));
        }
        let id = BlockId(parse_uuid(
            &required_map_string(&meta, BLOCK_ID)?,
            BLOCK_ID,
        )?);
        if !global_ids.insert(id) {
            return Err(CrdtError::DuplicateBlockId(id));
        }
        output.push(BlockSnapshot {
            id,
            parent: parent_id,
            order: u32::try_from(order)
                .map_err(|_| CrdtError::InvalidDocument("too many sibling blocks".into()))?,
            raw: required_text(&meta, RAW)?,
        });
        materialize_children(tree, node, Some(id), output, global_ids)?;
    }
    Ok(())
}

fn descendants(tree: &LoroTree, parent: TreeID) -> Vec<TreeID> {
    let mut output = Vec::new();
    let mut stack = tree.children(parent).unwrap_or_default();
    while let Some(node) = stack.pop() {
        output.push(node);
        stack.extend(tree.children(node).unwrap_or_default());
    }
    output
}

fn put_string(map: &LoroMap, key: &str, value: &str) -> Result<(), CrdtError> {
    map.insert(key, value).map_err(loro_error)
}

fn put_bool(map: &LoroMap, key: &str, value: bool) -> Result<(), CrdtError> {
    map.insert(key, value).map_err(loro_error)
}

fn replace_text(text: &LoroText, value: &str) -> Result<(), CrdtError> {
    if text.to_string() == value {
        return Ok(());
    }
    text.update(value, UpdateOptions::default())
        .map_err(loro_error)
}

fn required_string(tree: &LoroTree, node: TreeID, key: &str) -> Result<String, CrdtError> {
    required_map_string(&tree.get_meta(node).map_err(loro_error)?, key)
}

fn required_map_string(map: &LoroMap, key: &str) -> Result<String, CrdtError> {
    optional_string(map, key)?.ok_or_else(|| {
        CrdtError::InvalidDocument(format!("missing or non-string metadata field {key:?}"))
    })
}

fn optional_string(map: &LoroMap, key: &str) -> Result<Option<String>, CrdtError> {
    match map.get(key) {
        None => Ok(None),
        Some(ValueOrContainer::Value(LoroValue::String(value))) => Ok(Some((*value).clone())),
        Some(_) => Err(CrdtError::InvalidDocument(format!(
            "metadata field {key:?} is not a string"
        ))),
    }
}

fn required_text(map: &LoroMap, key: &str) -> Result<String, CrdtError> {
    match map.get(key) {
        Some(ValueOrContainer::Container(Container::Text(text))) => Ok(text.to_string()),
        _ => Err(CrdtError::InvalidDocument(format!(
            "missing mergeable text field {key:?}"
        ))),
    }
}

fn required_bool(map: &LoroMap, key: &str) -> Result<bool, CrdtError> {
    match map.get(key) {
        Some(ValueOrContainer::Value(LoroValue::Bool(value))) => Ok(value),
        _ => Err(CrdtError::InvalidDocument(format!(
            "missing boolean metadata field {key:?}"
        ))),
    }
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, CrdtError> {
    value.parse().map_err(|error| {
        CrdtError::InvalidDocument(format!("metadata field {field:?} is not a UUID: {error}"))
    })
}

fn loro_error(error: impl std::fmt::Display) -> CrdtError {
    CrdtError::Loro(error.to_string())
}
