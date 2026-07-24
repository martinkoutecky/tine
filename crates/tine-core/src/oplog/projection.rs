//! Production projection publication has no policy-explicit entry point:
//!
//! ```compile_fail
//! use tine_core::oplog::write_projection_with_policy;
//! ```
//!
//! Dense receipt policy selection is not part of the production API:
//!
//! ```compile_fail
//! use tine_core::oplog::ProjectionPolicy;
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::io;

use super::{
    AnnotatedIdentity, AnnotatedProjectionBase, BaseBlob, BatchInspection, BlockId, EngineError,
    LogseqIdentityOrigin, LogseqUuid, ManifestProjectionPrecondition, ManifestProjectionTarget,
    ManifestedProjectionIntent, MaterializedBlock, MaterializedPage, ObjectKind, ObjectStore,
    PageId, ProjectionCompletion, ProjectionEndpointId, ProjectionIntent, ProjectionPageState,
    ProjectionPrecondition, ProjectionReceiptStore, ProjectionStoreError, ProjectionWork,
    ProjectionWorkBlockAuthority, ProjectionWorkIndex, ProjectionWorkStatus, ReceiptError,
    ShardedHotEngine, StructuralLocator, StructuralSpan, WorkspaceId,
};
use crate::doc::{DocBlock, Document, SerializeOpts};
use crate::Graph;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectionFormat {
    Markdown,
    Org,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectionRenderMode {
    Sparse,
    #[cfg(test)]
    DenseInstrumentation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyGeneratedAnchor {
    block_id: BlockId,
    logseq_uuid: LogseqUuid,
}

impl PolicyGeneratedAnchor {
    pub const fn block_id(&self) -> BlockId {
        self.block_id
    }

    pub const fn logseq_uuid(&self) -> LogseqUuid {
        self.logseq_uuid
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPlan {
    intent: ProjectionIntent,
    base: Option<BaseBlob>,
    target: Vec<u8>,
    generated_anchors: Vec<PolicyGeneratedAnchor>,
}

impl ProjectionPlan {
    pub fn intent(&self) -> &ProjectionIntent {
        &self.intent
    }

    pub fn target(&self) -> &[u8] {
        &self.target
    }

    fn base(&self) -> Option<&BaseBlob> {
        self.base.as_ref()
    }

    pub fn generated_anchors(&self) -> &[PolicyGeneratedAnchor] {
        &self.generated_anchors
    }

    pub(crate) fn into_intent_and_target(self) -> (ProjectionIntent, Vec<u8>) {
        (self.intent, self.target)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionWrite {
    pub plan: ProjectionPlan,
    pub completion: ProjectionCompletion,
}

struct RenderedProjection {
    target: Vec<u8>,
    annotations: Vec<AnnotatedIdentity>,
    generated_anchors: Vec<PolicyGeneratedAnchor>,
}

/// Build exact projection bytes and receipt annotations without touching disk.
pub fn plan_projection(
    workspace_id: WorkspaceId,
    state: &ProjectionPageState,
    expected_base: Option<&[u8]>,
) -> Result<ProjectionPlan, ProjectionError> {
    let rendered = render_projection(state, expected_base)?;
    let base = expected_base.map(|bytes| BaseBlob::new(bytes.to_vec()));
    let precondition = base
        .as_ref()
        .map_or(ProjectionPrecondition::Absent, |base| {
            ProjectionPrecondition::Base(base.description())
        });
    let intent = ProjectionIntent::new(
        workspace_id,
        state.page.page_id,
        state.page.path.clone(),
        state.frontier.clone(),
        state.claim_evidence.clone(),
        precondition,
        super::BlobDescription::of(&rendered.target),
        rendered.annotations,
    )?;
    Ok(ProjectionPlan {
        intent,
        base,
        target: rendered.target,
        generated_anchors: rendered.generated_anchors,
    })
}

fn render_projection(
    state: &ProjectionPageState,
    expected_base: Option<&[u8]>,
) -> Result<RenderedProjection, ProjectionError> {
    let format = format_for_page(&state.page)?;
    let base_text = expected_base
        .map(|bytes| {
            std::str::from_utf8(bytes).map_err(|_| ProjectionError::InvalidUtf8("projection base"))
        })
        .transpose()?;
    let mut metadata = ProjectionMetadata::with_capacity(state.page.blocks.len());
    let document = build_projection_document(
        state,
        format,
        ProjectionRenderMode::Sparse,
        Some(&mut metadata),
    )?;
    let target = serialize_document(format, &document, base_text).into_bytes();
    let annotations = annotate_serialized_blocks(
        format,
        &document,
        base_text,
        &target,
        &metadata.pending_annotations,
    )?;
    metadata
        .generated_anchors
        .sort_unstable_by_key(PolicyGeneratedAnchor::block_id);
    Ok(RenderedProjection {
        target,
        annotations,
        generated_anchors: metadata.generated_anchors,
    })
}

#[cfg(test)]
fn render_dense_projection_bytes(
    state: &ProjectionPageState,
    expected_base: Option<&[u8]>,
) -> Result<Vec<u8>, ProjectionError> {
    let format = format_for_page(&state.page)?;
    let base_text = expected_base
        .map(|bytes| {
            std::str::from_utf8(bytes).map_err(|_| ProjectionError::InvalidUtf8("projection base"))
        })
        .transpose()?;
    let document = build_projection_document(
        state,
        format,
        ProjectionRenderMode::DenseInstrumentation,
        None,
    )?;
    Ok(serialize_document(format, &document, base_text).into_bytes())
}

fn build_projection_document(
    state: &ProjectionPageState,
    format: ProjectionFormat,
    mode: ProjectionRenderMode,
    mut metadata: Option<&mut ProjectionMetadata>,
) -> Result<Document, ProjectionError> {
    let forest = ValidatedForest::new(&state.page.blocks)?;
    let raw_ids = collect_raw_logseq_ids(&state.page.blocks, format);
    validate_logseq_state(&state.page.blocks, &raw_ids)?;

    let mut roots = Vec::with_capacity(forest.roots.len());
    for (root_position, index) in forest.roots.iter().copied().enumerate() {
        roots.push(build_doc_block(
            &state.page.blocks,
            &forest,
            index,
            vec![u32_index(root_position)?],
            format,
            mode,
            &raw_ids,
            metadata.as_deref_mut(),
        )?);
    }

    Ok(Document {
        pre_block: state.page.preamble.clone(),
        roots,
    })
}

/// Derive a receiver-local receipt intent from accepted semantic state and
/// that receiver's exact local bytes. The source intent supplies no write
/// authority and its portable target bytes are deliberately not reused.
pub fn derive_receiver_local_projection(
    engine: &ShardedHotEngine,
    source: &ManifestedProjectionIntent,
    receiver_endpoint_id: ProjectionEndpointId,
    exact_local_base: Option<&[u8]>,
) -> Result<ProjectionPlan, ProjectionError> {
    if source.workspace_id() != engine.workspace_id() {
        return Err(ProjectionError::ReceiverSourceMismatch);
    }
    if source.source_endpoint_id() == receiver_endpoint_id {
        return Err(ProjectionError::ReceiverEndpointIsSource);
    }
    if !matches!(source.target(), ManifestProjectionTarget::Present { .. }) {
        return Err(ProjectionError::ReceiverSourceAbsent);
    }
    let authorization = engine.authorize_projection_recovery(
        source.page_id(),
        source.post_frontier(),
        source.claim_evidence(),
    )?;
    plan_projection(
        engine.workspace_id(),
        authorization.state(),
        exact_local_base,
    )
}

/// Execute one source-endpoint manifested work row. Recovery reloads immutable
/// target/base objects through the batch locator; no local intent or target
/// copy is published. Device-local attempt reservations remain forensic, while
/// stable completion is recorded against the immutable work/object reference.
pub fn execute_manifested_projection_work(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &mut ShardedHotEngine,
    work: &ProjectionWork,
) -> Result<(), ProjectionError> {
    let (archive, work_index) = engine.enrolled_projection_runtime()?;
    execute_manifested_projection_work_with_runtime(
        graph,
        receipts,
        &archive,
        engine,
        &work_index,
        work,
    )
}

fn execute_manifested_projection_work_with_runtime(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    archive: &ObjectStore,
    engine: &mut ShardedHotEngine,
    work_index: &ProjectionWorkIndex,
    work: &ProjectionWork,
) -> Result<(), ProjectionError> {
    let endpoint = engine
        .projection_endpoint_binding()
        .ok_or(ProjectionError::EndpointBindingMismatch)?;
    let receipt_store_id = engine
        .projection_receipt_store_id()
        .ok_or(ProjectionError::EndpointBindingMismatch)?;
    if receipts.store_id() != receipt_store_id || work_index.receipt_store_id() != receipt_store_id
    {
        return Err(ProjectionError::EndpointBindingMismatch);
    }
    receipts.require_endpoint(endpoint)?;
    if graph.canonical_resource_id()? != endpoint.graph_resource_id {
        return Err(ProjectionError::EndpointBindingMismatch);
    }
    engine
        .authorize_projection_work(work_index, work)
        .map_err(ProjectionError::Engine)?;
    if work_index
        .status(work.work_id())
        .map_err(|error| ProjectionError::Work(error.to_string()))?
        != Some(ProjectionWorkStatus::Ready)
    {
        return Err(ProjectionError::WorkNotReady);
    }
    let batch = match archive
        .inspect_batch(work.batch_id())
        .map_err(|error| ProjectionError::Archive(error.to_string()))?
    {
        BatchInspection::Ready(batch) => batch,
        BatchInspection::Absent | BatchInspection::Staged { .. } => {
            return Err(ProjectionError::Archive(
                "projection work batch is not a complete immutable object set".into(),
            ));
        }
    };
    let intent_object = batch
        .objects()
        .iter()
        .find(|object| {
            object.kind() == ObjectKind::ProjectionIntent
                && object.document_id() == work.intent().document_id()
                && object.descriptor().is_ok_and(|descriptor| {
                    descriptor.content_digest() == work.intent().content_digest()
                        && descriptor.encoded_byte_length() == work.intent().encoded_byte_length()
                })
        })
        .ok_or(ProjectionError::WorkIntentMismatch)?;
    let manifested = ManifestedProjectionIntent::decode(intent_object.payload())
        .map_err(|error| ProjectionError::Archive(error.to_string()))?;
    if manifested.source_endpoint_id() != work.endpoint_id()
        || manifested.page_id() != work.page_id()
        || manifested.path() != work.path()
        || manifested.post_frontier() != work.post_frontier()
    {
        return Err(ProjectionError::WorkIntentMismatch);
    }
    let (description, target, annotations) = match manifested.target() {
        ManifestProjectionTarget::Absent => (super::BlobDescription::of(&[]), None, Vec::new()),
        ManifestProjectionTarget::Present {
            description,
            bytes,
            annotations,
        } => (*description, Some(bytes.as_slice()), annotations.clone()),
    };
    let expected_base = match manifested.precondition() {
        ManifestProjectionPrecondition::Absent => None,
        ManifestProjectionPrecondition::Present { base } => {
            let base_object = batch
                .objects()
                .iter()
                .find(|object| {
                    object.kind() == ObjectKind::AnnotatedBaseBlob
                        && object.document_id() == base.document_id()
                        && object.descriptor().is_ok_and(|descriptor| {
                            descriptor.content_digest() == base.content_digest()
                                && descriptor.encoded_byte_length() == base.encoded_byte_length()
                        })
                })
                .ok_or(ProjectionError::WorkIntentMismatch)?;
            Some(
                AnnotatedProjectionBase::decode(base_object.payload())
                    .map_err(|error| ProjectionError::Archive(error.to_string()))?,
            )
        }
    };
    let local_attempt_intent = ProjectionIntent::new(
        manifested.workspace_id(),
        manifested.page_id(),
        manifested.path().clone(),
        manifested.post_frontier().clone(),
        manifested.claim_evidence().to_vec(),
        expected_base
            .as_ref()
            .map_or(ProjectionPrecondition::Absent, |base| {
                ProjectionPrecondition::Base(base.description())
            }),
        description,
        annotations,
    )?;
    receipts.publish_intent(
        &local_attempt_intent,
        expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
    )?;
    if receipts.load_completion(&local_attempt_intent)?.is_some() {
        let authority = receipts.completed_work_authority(work, &local_attempt_intent)?;
        work_index
            .mark_completed(authority)
            .map_err(|error| ProjectionError::Work(error.to_string()))?;
        return Ok(());
    }
    let attempts = receipts.load_attempt_reservations(&local_attempt_intent)?;
    let has_attempts = !attempts.is_empty();
    let recovery_result = if !has_attempts {
        None
    } else {
        let mut authority = receipts.begin_mutation(&local_attempt_intent, None)?;
        let result = match target {
            Some(target) => graph.recover_page_projection(
                manifested.path().as_str(),
                expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                target,
                &mut authority,
            ),
            None => {
                let base = expected_base
                    .as_ref()
                    .ok_or(ProjectionError::WorkIntentMismatch)?;
                graph.recover_removed_page_projection(
                    manifested.path().as_str(),
                    base.bytes(),
                    &mut authority,
                )
            }
        };
        Some((result, authority))
    };
    let recovered = match recovery_result {
        Some((Ok(proof), authority)) => Some((proof, authority)),
        Some((Err(error), authority))
            if matches!(
                error.kind(),
                io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
            ) =>
        {
            authority.release_failed_recovery()?;
            None
        }
        Some((Err(error), _)) => return Err(error.into()),
        None => None,
    };
    let (proof, authority) = match recovered {
        Some(recovered) => recovered,
        None => {
            let mut authority = if has_attempts {
                let reservation = receipts.reserve_fallback_attempt(&local_attempt_intent)?;
                receipts.begin_mutation(&local_attempt_intent, Some(&reservation))?
            } else {
                let reservation = receipts.reserve_attempt(&local_attempt_intent)?;
                receipts.begin_mutation(&local_attempt_intent, Some(&reservation))?
            };
            let write_result = match target {
                Some(target) => graph.write_page_projection(
                    manifested.path().as_str(),
                    expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                    target,
                    &mut authority,
                ),
                None => {
                    let base = expected_base
                        .as_ref()
                        .ok_or(ProjectionError::WorkIntentMismatch)?;
                    graph.remove_page_projection(
                        manifested.path().as_str(),
                        base.bytes(),
                        &mut authority,
                    )
                }
            };
            match write_result {
                Ok(proof) => (proof, authority),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
                    ) =>
                {
                    let observed = graph
                        .read_projection_input(work.path())
                        .map_err(ProjectionError::Io)?
                        .as_deref()
                        .map(super::BlobDescription::of);
                    work_index
                        .mark_blocked(ProjectionWorkBlockAuthority::guarded_conflict(
                            work,
                            receipts.store_id(),
                            observed,
                        ))
                        .map_err(|error| ProjectionError::Work(error.to_string()))?;
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            }
        }
    };
    receipts.publish_completion(authority, &local_attempt_intent, &proof)?;
    let authority = receipts.completed_work_authority(work, &local_attempt_intent)?;
    work_index
        .mark_completed(authority)
        .map_err(|error| ProjectionError::Work(error.to_string()))?;
    Ok(())
}

/// Publish intent/base evidence, invoke the singular guarded graph writer, and
/// publish completion only after the writer returns the exact reread target.
pub fn write_projection_exact(
    graph: &Graph,
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    page_id: PageId,
    expected_base: Option<&[u8]>,
) -> Result<ProjectionWrite, ProjectionError> {
    require_endpoint_authority(graph, store, engine)?;
    let authorization = engine.authorize_projection_write(page_id)?;
    let plan = plan_projection(engine.workspace_id(), authorization.state(), expected_base)?;
    store.publish_intent(plan.intent(), plan.base().map(BaseBlob::bytes))?;
    let reservation = store.reserve_attempt(plan.intent())?;
    let mut authority = store.begin_mutation(plan.intent(), Some(&reservation))?;
    let proof = graph.write_page_projection(
        plan.intent().path().as_str(),
        expected_base,
        plan.target(),
        &mut authority,
    )?;
    let completion = store.publish_completion(authority, plan.intent(), &proof)?;
    debug_assert_eq!(authorization.state().page.page_id, page_id);
    Ok(ProjectionWrite { plan, completion })
}

/// Recover every incomplete intent only when current accepted engine state
/// replays the exact intent and Graph freshly proves that exact target durable.
pub fn recover_incomplete_projections(
    graph: &Graph,
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
) -> Result<Vec<ProjectionWrite>, ProjectionError> {
    require_endpoint_authority(graph, store, engine)?;
    let mut recovered = Vec::new();
    for intent in store.incomplete_intents()? {
        let authorization = engine.authorize_projection_recovery(
            intent.page_id(),
            intent.frontier(),
            intent.claim_evidence(),
        )?;
        let base = store.load_base(&intent)?;
        let expected_base = base.as_ref().map(BaseBlob::bytes);
        let plan = plan_projection(engine.workspace_id(), authorization.state(), expected_base)?;
        if plan.intent() != &intent {
            return Err(ProjectionError::RecoveryIntentMismatch);
        }
        let attempts = store.load_attempt_reservations(&intent)?;
        let recovery_attempt = if attempts.is_empty() {
            None
        } else {
            let mut authority = store.begin_mutation(&intent, None)?;
            let result = graph.recover_page_projection(
                intent.path().as_str(),
                expected_base,
                plan.target(),
                &mut authority,
            );
            Some((result, authority))
        };
        let (proof, authority) = match recovery_attempt {
            Some((Ok(proof), authority)) => (proof, authority),
            None => {
                let mut recovery_authority = store.begin_mutation(&intent, None)?;
                match graph.recover_page_projection(
                    intent.path().as_str(),
                    expected_base,
                    plan.target(),
                    &mut recovery_authority,
                ) {
                    Ok(proof) => (proof, recovery_authority),
                    Err(recovery_error)
                        if matches!(
                            recovery_error.kind(),
                            io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
                        ) =>
                    {
                        recovery_authority.release_failed_recovery()?;
                        let reservation = store.reserve_fallback_attempt(&intent)?;
                        let mut write_authority =
                            store.begin_mutation(&intent, Some(&reservation))?;
                        let proof = graph.write_page_projection(
                            intent.path().as_str(),
                            expected_base,
                            plan.target(),
                            &mut write_authority,
                        )?;
                        (proof, write_authority)
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Some((Err(recovery_error), recovery_authority))
                if matches!(
                    recovery_error.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
                ) =>
            {
                recovery_authority.release_failed_recovery()?;
                let reservation = store.reserve_fallback_attempt(&intent)?;
                let mut authority = store.begin_mutation(&intent, Some(&reservation))?;
                let proof = graph.write_page_projection(
                    intent.path().as_str(),
                    expected_base,
                    plan.target(),
                    &mut authority,
                )?;
                (proof, authority)
            }
            Some((Err(error), _)) => return Err(error.into()),
        };
        let completion = store.reconstruct_completion(authority, &intent, plan.target(), &proof)?;
        debug_assert_eq!(authorization.state().page.page_id, intent.page_id());
        recovered.push(ProjectionWrite { plan, completion });
    }
    Ok(recovered)
}

fn require_endpoint_authority(
    graph: &Graph,
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
) -> Result<super::ProjectionEndpointBinding, ProjectionError> {
    let endpoint = engine
        .projection_endpoint_binding()
        .ok_or(ProjectionError::EndpointBindingMismatch)?;
    if engine.projection_receipt_store_id() != Some(store.store_id()) {
        return Err(ProjectionError::EndpointBindingMismatch);
    }
    store.require_endpoint(endpoint)?;
    if graph.canonical_resource_id()? != endpoint.graph_resource_id {
        return Err(ProjectionError::EndpointBindingMismatch);
    }
    Ok(endpoint)
}

struct PendingAnnotation {
    locator: Vec<u32>,
    block_id: BlockId,
    logseq_uuid: Option<LogseqUuid>,
    raw_is_empty: bool,
}

struct ProjectionMetadata {
    pending_annotations: Vec<PendingAnnotation>,
    generated_anchors: Vec<PolicyGeneratedAnchor>,
}

impl ProjectionMetadata {
    fn with_capacity(block_count: usize) -> Self {
        Self {
            pending_annotations: Vec::with_capacity(block_count),
            generated_anchors: Vec::new(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_doc_block(
    blocks: &[MaterializedBlock],
    forest: &ValidatedForest,
    index: usize,
    locator: Vec<u32>,
    format: ProjectionFormat,
    mode: ProjectionRenderMode,
    raw_ids: &RawIdOwners,
    mut metadata: Option<&mut ProjectionMetadata>,
) -> Result<DocBlock, ProjectionError> {
    let block = &blocks[index];
    let (content, projected_uuid, generated) = project_block_content(block, format, mode, raw_ids)?;
    if let Some(metadata) = metadata.as_deref_mut() {
        if generated {
            metadata.generated_anchors.push(PolicyGeneratedAnchor {
                block_id: block.block_id,
                logseq_uuid: projected_uuid.expect("generated anchor has a UUID"),
            });
        }
        metadata.pending_annotations.push(PendingAnnotation {
            locator: locator.clone(),
            block_id: block.block_id,
            logseq_uuid: projected_uuid,
            raw_is_empty: content.is_empty(),
        });
    }

    let mut projected = DocBlock::new(content);
    projected.uuid = block.block_id.to_string();
    projected.is_org = format == ProjectionFormat::Org;
    if let Some(children) = forest.children.get(&block.block_id) {
        projected.children.reserve(children.len());
        for (child_position, child_index) in children.iter().copied().enumerate() {
            let mut child_locator = locator.clone();
            child_locator.push(u32_index(child_position)?);
            projected.children.push(build_doc_block(
                blocks,
                forest,
                child_index,
                child_locator,
                format,
                mode,
                raw_ids,
                metadata.as_deref_mut(),
            )?);
        }
    }
    Ok(projected)
}

struct ValidatedForest {
    roots: Vec<usize>,
    children: BTreeMap<BlockId, Vec<usize>>,
}

impl ValidatedForest {
    fn new(blocks: &[MaterializedBlock]) -> Result<Self, ProjectionError> {
        let mut indexes = HashMap::with_capacity(blocks.len());
        for (index, block) in blocks.iter().enumerate() {
            if indexes.insert(block.block_id, index).is_some() {
                return Err(ProjectionError::DuplicateBlock(block.block_id));
            }
            if block.order.is_empty() {
                return Err(ProjectionError::EmptyOrder(block.block_id));
            }
        }

        let mut roots = Vec::new();
        let mut children = BTreeMap::<BlockId, Vec<usize>>::new();
        for (index, block) in blocks.iter().enumerate() {
            match block.parent {
                None => roots.push(index),
                Some(parent) if parent == block.block_id => {
                    return Err(ProjectionError::CyclicTree(block.block_id));
                }
                Some(parent) if indexes.contains_key(&parent) => {
                    children.entry(parent).or_default().push(index);
                }
                Some(parent) => {
                    return Err(ProjectionError::MissingParent {
                        block: block.block_id,
                        parent,
                    });
                }
            }
        }
        sort_siblings(blocks, None, &mut roots)?;
        for (parent, siblings) in &mut children {
            sort_siblings(blocks, Some(*parent), siblings)?;
        }

        let mut visited = BTreeSet::new();
        let mut stack = roots.clone();
        while let Some(index) = stack.pop() {
            let id = blocks[index].block_id;
            if !visited.insert(id) {
                return Err(ProjectionError::CyclicTree(id));
            }
            if let Some(descendants) = children.get(&id) {
                stack.extend(descendants.iter().copied());
            }
        }
        if visited.len() != blocks.len() {
            let block = blocks
                .iter()
                .find(|block| !visited.contains(&block.block_id))
                .expect("unvisited block exists")
                .block_id;
            return Err(ProjectionError::CyclicTree(block));
        }
        Ok(Self { roots, children })
    }
}

fn sort_siblings(
    blocks: &[MaterializedBlock],
    parent: Option<BlockId>,
    siblings: &mut [usize],
) -> Result<(), ProjectionError> {
    siblings.sort_unstable_by(|left, right| {
        (&blocks[*left].order, blocks[*left].block_id)
            .cmp(&(&blocks[*right].order, blocks[*right].block_id))
    });
    if let Some(pair) = siblings
        .windows(2)
        .find(|pair| blocks[pair[0]].order == blocks[pair[1]].order)
    {
        return Err(ProjectionError::DuplicateSiblingOrder {
            parent,
            order: blocks[pair[0]].order.clone(),
        });
    }
    Ok(())
}

type RawIdOwners = BTreeMap<LogseqUuid, Vec<BlockId>>;

fn collect_raw_logseq_ids(blocks: &[MaterializedBlock], format: ProjectionFormat) -> RawIdOwners {
    let mut owners = RawIdOwners::new();
    for block in blocks {
        let mut parsed = DocBlock::new(&block.content);
        parsed.is_org = format == ProjectionFormat::Org;
        for uuid in parsed
            .projection()
            .properties
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("id"))
            .filter_map(|(_, value)| value.trim().parse().ok())
        {
            owners.entry(uuid).or_default().push(block.block_id);
        }
    }
    owners
}

fn validate_logseq_state(
    blocks: &[MaterializedBlock],
    raw_ids: &RawIdOwners,
) -> Result<(), ProjectionError> {
    let mut claims = BTreeSet::new();
    for block in blocks {
        if block.logseq_uuid.is_some() != block.logseq_identity_origin.is_some() {
            return Err(ProjectionError::InconsistentLogseqIdentityOrigin(
                block.block_id,
            ));
        }
        let Some(uuid) = block.logseq_uuid else {
            continue;
        };
        if !claims.insert(uuid) {
            return Err(ProjectionError::DuplicateLogseqClaim(uuid));
        }
        if let Some(owners) = raw_ids.get(&uuid) {
            if owners.len() != 1 || owners[0] != block.block_id {
                return Err(ProjectionError::AmbiguousRawLogseqId(uuid));
            }
        }
    }
    Ok(())
}

fn project_block_content(
    block: &MaterializedBlock,
    format: ProjectionFormat,
    mode: ProjectionRenderMode,
    raw_ids: &RawIdOwners,
) -> Result<(String, Option<LogseqUuid>, bool), ProjectionError> {
    let desired_uuid = match (block.logseq_uuid, block.logseq_identity_origin, mode) {
        (None, None, ProjectionRenderMode::Sparse) => {
            return Ok((block.content.clone(), None, false));
        }
        #[cfg(test)]
        (None, None, ProjectionRenderMode::DenseInstrumentation) => {
            LogseqUuid::from_uuid(block.block_id.as_uuid())
        }
        (Some(uuid), Some(_), _) => uuid,
        _ => {
            return Err(ProjectionError::InconsistentLogseqIdentityOrigin(
                block.block_id,
            ));
        }
    };
    match raw_ids.get(&desired_uuid) {
        Some(owners) if owners.len() == 1 && owners[0] == block.block_id => {
            Ok((block.content.clone(), Some(desired_uuid), false))
        }
        Some(_) => Err(ProjectionError::AmbiguousRawLogseqId(desired_uuid)),
        None if matches!(
            block.logseq_identity_origin,
            Some(LogseqIdentityOrigin::ExternalImported)
        ) =>
        {
            Err(ProjectionError::MissingExternalRawLogseqId {
                block: block.block_id,
                logseq_uuid: desired_uuid,
            })
        }
        None => Ok((
            inject_logseq_id(&block.content, format, desired_uuid)?,
            Some(desired_uuid),
            true,
        )),
    }
}

fn inject_logseq_id(
    content: &str,
    format: ProjectionFormat,
    uuid: LogseqUuid,
) -> Result<String, ProjectionError> {
    match format {
        ProjectionFormat::Markdown => {
            if content.is_empty() {
                Ok(format!("\nid:: {uuid}"))
            } else {
                Ok(format!("{content}\nid:: {uuid}"))
            }
        }
        ProjectionFormat::Org => inject_org_id(content, uuid),
    }
}

fn inject_org_id(content: &str, uuid: LogseqUuid) -> Result<String, ProjectionError> {
    let projection = crate::render::parse_projection(content, true);
    if let Some(span) = projection.blocks.iter().find_map(|block| match block {
        lsdoc::ast::Block::Properties {
            span: Some(span), ..
        } => Some(span),
        _ => None,
    }) {
        let lead = content.len() - content.trim_start().len();
        let start = span.0.saturating_sub(2).saturating_add(lead);
        let end = span.1.saturating_sub(2).saturating_add(lead);
        let drawer = content
            .get(start.min(content.len())..end.min(content.len()))
            .ok_or(ProjectionError::ParserSpanMismatch)?;
        let mut close_offset = None;
        let mut offset = 0;
        for segment in drawer.split_inclusive('\n') {
            let line = segment.trim_end_matches('\n');
            if line.trim().eq_ignore_ascii_case(":END:") {
                close_offset = Some(offset);
                break;
            }
            offset += segment.len();
        }
        let close_offset = close_offset.ok_or(ProjectionError::ParserSpanMismatch)?;
        let insertion = start + close_offset;
        let indent = drawer[..close_offset]
            .rsplit_once('\n')
            .map_or(&drawer[..close_offset], |(_, line)| line);
        let indent = &indent[..indent.len() - indent.trim_start().len()];
        let mut result = String::with_capacity(content.len() + uuid.to_string().len() + 7);
        result.push_str(&content[..insertion]);
        result.push_str(indent);
        result.push_str(":id: ");
        result.push_str(&uuid.to_string());
        result.push('\n');
        result.push_str(&content[insertion..]);
        return Ok(result);
    }

    let lines: Vec<&str> = content.split('\n').collect();
    let mut insert_at = 1.min(lines.len());
    while insert_at < lines.len() && is_org_planning_line(lines[insert_at]) {
        insert_at += 1;
    }
    let mut output = Vec::with_capacity(lines.len() + 3);
    output.extend_from_slice(&lines[..insert_at]);
    output.push(":PROPERTIES:");
    let id = format!(":id: {uuid}");
    output.push(&id);
    output.push(":END:");
    output.extend_from_slice(&lines[insert_at..]);
    Ok(output.join("\n"))
}

fn is_org_planning_line(line: &str) -> bool {
    let line = line.trim_start();
    ["SCHEDULED:", "DEADLINE:", "CLOSED:"]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

fn format_for_page(page: &MaterializedPage) -> Result<ProjectionFormat, ProjectionError> {
    match page.path.as_str().rsplit_once('.') {
        Some((_, "md")) => Ok(ProjectionFormat::Markdown),
        Some((_, "org")) => Ok(ProjectionFormat::Org),
        _ => Err(ProjectionError::UnsupportedFormat(
            page.path.as_str().into(),
        )),
    }
}

fn u32_index(value: usize) -> Result<u32, ProjectionError> {
    u32::try_from(value).map_err(|_| ProjectionError::TreeTooWide)
}

fn serialize_document(format: ProjectionFormat, document: &Document, base: Option<&str>) -> String {
    let serialized = match format {
        ProjectionFormat::Markdown => {
            crate::doc::serialize_with(document, &SerializeOpts::detect(base))
        }
        ProjectionFormat::Org => crate::org::serialize_org_detect(document, base),
    };
    if base.is_some_and(|text| text.contains("\r\n")) {
        serialized.replace('\n', "\r\n")
    } else {
        serialized
    }
}

fn annotate_serialized_blocks(
    format: ProjectionFormat,
    document: &Document,
    base: Option<&str>,
    target: &[u8],
    pending: &[PendingAnnotation],
) -> Result<Vec<AnnotatedIdentity>, ProjectionError> {
    let mut salt = 0_u64;
    let marker_prefix = loop {
        let candidate = format!("\u{1e}TINE-PROJECTION-SPAN-{salt:016x}-");
        if !target
            .windows(candidate.len())
            .any(|window| window == candidate.as_bytes())
        {
            break candidate;
        }
        salt = salt
            .checked_add(1)
            .ok_or(ProjectionError::SpanInstrumentationMismatch)?;
    };

    let mut marked = document.clone();
    let mut marked_count = 0;
    mark_document_blocks(
        &mut marked.roots,
        pending,
        &marker_prefix,
        &mut marked_count,
    )?;
    if marked_count != pending.len() {
        return Err(ProjectionError::SpanInstrumentationMismatch);
    }
    let marked_bytes = serialize_document(format, &marked, base).into_bytes();

    let mut clean = Vec::with_capacity(target.len());
    let mut cursor = 0;
    let mut annotations = Vec::with_capacity(pending.len());
    for (index, annotation) in pending.iter().enumerate() {
        let start_marker = span_marker(&marker_prefix, index, 'S');
        let end_marker = span_marker(&marker_prefix, index, 'E');
        let start_at = find_bytes(&marked_bytes, start_marker.as_bytes(), cursor)
            .ok_or(ProjectionError::SpanInstrumentationMismatch)?;
        clean.extend_from_slice(&marked_bytes[cursor..start_at]);
        let span_start = clean
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        cursor = start_at + start_marker.len();

        if annotation.raw_is_empty {
            if !marked_bytes[cursor..].starts_with(end_marker.as_bytes())
                || clean.pop() != Some(b' ')
            {
                return Err(ProjectionError::SpanInstrumentationMismatch);
            }
        } else {
            let end_at = find_bytes(&marked_bytes, end_marker.as_bytes(), cursor)
                .ok_or(ProjectionError::SpanInstrumentationMismatch)?;
            clean.extend_from_slice(&marked_bytes[cursor..end_at]);
            cursor = end_at;
        }
        if !marked_bytes[cursor..].starts_with(end_marker.as_bytes()) {
            return Err(ProjectionError::SpanInstrumentationMismatch);
        }
        cursor += end_marker.len();

        annotations.push(AnnotatedIdentity::new(
            StructuralLocator::new(annotation.locator.clone())?,
            StructuralSpan::new(
                u64::try_from(span_start).map_err(|_| ProjectionError::ProjectionTooLarge)?,
                u64::try_from(clean.len()).map_err(|_| ProjectionError::ProjectionTooLarge)?,
            )?,
            annotation.block_id,
            annotation.logseq_uuid,
        ));
    }
    clean.extend_from_slice(&marked_bytes[cursor..]);
    if clean != target {
        return Err(ProjectionError::SpanInstrumentationMismatch);
    }
    Ok(annotations)
}

fn mark_document_blocks(
    blocks: &mut [DocBlock],
    pending: &[PendingAnnotation],
    marker_prefix: &str,
    index: &mut usize,
) -> Result<(), ProjectionError> {
    for block in blocks {
        let annotation = pending
            .get(*index)
            .ok_or(ProjectionError::SpanInstrumentationMismatch)?;
        if block.raw.is_empty() != annotation.raw_is_empty {
            return Err(ProjectionError::SpanInstrumentationMismatch);
        }
        let start = span_marker(marker_prefix, *index, 'S');
        let end = span_marker(marker_prefix, *index, 'E');
        block.raw = format!("{start}{}{end}", block.raw);
        *index += 1;
        mark_document_blocks(&mut block.children, pending, marker_prefix, index)?;
    }
    Ok(())
}

fn span_marker(prefix: &str, index: usize, side: char) -> String {
    format!("{prefix}{index:016x}-{side}\u{1f}")
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

#[derive(Debug)]
pub enum ProjectionError {
    Io(io::Error),
    Engine(EngineError),
    Receipt(ReceiptError),
    Store(Box<ProjectionStoreError>),
    InvalidUtf8(&'static str),
    UnsupportedFormat(String),
    DuplicateBlock(BlockId),
    MissingParent {
        block: BlockId,
        parent: BlockId,
    },
    CyclicTree(BlockId),
    EmptyOrder(BlockId),
    DuplicateSiblingOrder {
        parent: Option<BlockId>,
        order: String,
    },
    DuplicateLogseqClaim(LogseqUuid),
    AmbiguousRawLogseqId(LogseqUuid),
    InconsistentLogseqIdentityOrigin(BlockId),
    MissingExternalRawLogseqId {
        block: BlockId,
        logseq_uuid: LogseqUuid,
    },
    ParserSpanMismatch,
    SpanInstrumentationMismatch,
    TreeTooWide,
    ProjectionTooLarge,
    RecoveryIntentMismatch,
    ReceiverSourceMismatch,
    ReceiverEndpointIsSource,
    ReceiverSourceAbsent,
    EndpointBindingMismatch,
    Archive(String),
    Work(String),
    WorkNotReady,
    WorkIntentMismatch,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Engine(error) => error.fmt(f),
            Self::Receipt(error) => error.fmt(f),
            Self::Store(error) => error.fmt(f),
            Self::InvalidUtf8(kind) => write!(f, "{kind} is not valid UTF-8"),
            Self::UnsupportedFormat(path) => {
                write!(f, "unsupported projection page format: {path}")
            }
            Self::DuplicateBlock(block) => write!(f, "duplicate materialized block {block}"),
            Self::MissingParent { block, parent } => {
                write!(f, "block {block} names missing parent {parent}")
            }
            Self::CyclicTree(block) => write!(f, "materialized hierarchy cycles at {block}"),
            Self::EmptyOrder(block) => write!(f, "block {block} has an empty order key"),
            Self::DuplicateSiblingOrder { parent, order } => {
                write!(f, "duplicate sibling order {order:?} below {parent:?}")
            }
            Self::DuplicateLogseqClaim(uuid) => {
                write!(f, "duplicate materialized Logseq UUID claim {uuid}")
            }
            Self::AmbiguousRawLogseqId(uuid) => {
                write!(f, "raw Logseq UUID {uuid} is ambiguous")
            }
            Self::InconsistentLogseqIdentityOrigin(block) => {
                write!(f, "block {block} has inconsistent Logseq identity origin")
            }
            Self::MissingExternalRawLogseqId { block, logseq_uuid } => {
                write!(
                    f,
                    "external/imported Logseq UUID {logseq_uuid} is not raw metadata on block {block}"
                )
            }
            Self::ParserSpanMismatch => {
                f.write_str("lsdoc property span does not map to authoritative block bytes")
            }
            Self::SpanInstrumentationMismatch => {
                f.write_str("serialized projection spans do not reconstruct exact target bytes")
            }
            Self::TreeTooWide => f.write_str("materialized hierarchy exceeds locator width"),
            Self::ProjectionTooLarge => f.write_str("projection exceeds receipt span range"),
            Self::RecoveryIntentMismatch => {
                f.write_str("accepted engine replay does not match incomplete projection intent")
            }
            Self::ReceiverSourceMismatch => {
                f.write_str("receiver and source projection workspaces do not match")
            }
            Self::ReceiverEndpointIsSource => {
                f.write_str("receiver-local derivation requires a non-source endpoint")
            }
            Self::ReceiverSourceAbsent => {
                f.write_str("receiver-local Present projection cannot derive from an Absent target")
            }
            Self::EndpointBindingMismatch => {
                f.write_str("projection endpoint is not enrolled to this graph capability")
            }
            Self::Archive(error) => write!(f, "immutable projection archive failed: {error}"),
            Self::Work(error) => write!(f, "projection work index failed: {error}"),
            Self::WorkNotReady => f.write_str("projection work is not ready"),
            Self::WorkIntentMismatch => {
                f.write_str("projection work does not match its immutable intent/base objects")
            }
        }
    }
}

impl std::error::Error for ProjectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Engine(error) => Some(error),
            Self::Receipt(error) => Some(error),
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ProjectionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<EngineError> for ProjectionError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl From<ReceiptError> for ProjectionError {
    fn from(error: ReceiptError) -> Self {
        Self::Receipt(error)
    }
}

impl From<ProjectionStoreError> for ProjectionError {
    fn from(error: ProjectionStoreError) -> Self {
        Self::Store(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::oplog::{DocumentId, FrontierV2, ManagedPath, MaterializationStats};

    #[derive(Debug, Eq, PartialEq)]
    struct CanonicalDocument {
        preamble: Option<String>,
        roots: Vec<CanonicalBlock>,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct CanonicalBlock {
        visible: String,
        properties: Vec<(String, String)>,
        children: Vec<CanonicalBlock>,
    }

    fn canonical_semantics(
        bytes: &[u8],
        instrumentation_generated_id: Option<LogseqUuid>,
    ) -> CanonicalDocument {
        let instrumentation_generated_id =
            instrumentation_generated_id.map(|uuid| uuid.to_string());
        let document = crate::doc::parse(std::str::from_utf8(bytes).unwrap());
        CanonicalDocument {
            preamble: document.pre_block,
            roots: canonical_blocks(&document.roots, instrumentation_generated_id.as_deref()),
        }
    }

    fn canonical_blocks(
        blocks: &[crate::doc::DocBlock],
        instrumentation_generated_id: Option<&str>,
    ) -> Vec<CanonicalBlock> {
        blocks
            .iter()
            .map(|block| CanonicalBlock {
                visible: block.projection().visible.clone(),
                properties: block
                    .projection()
                    .properties
                    .iter()
                    .filter(|(key, value)| {
                        !key.eq_ignore_ascii_case("id")
                            || instrumentation_generated_id != Some(value.trim())
                    })
                    .cloned()
                    .collect(),
                children: canonical_blocks(&block.children, instrumentation_generated_id),
            })
            .collect()
    }

    #[test]
    fn dense_bytes_and_sparse_projection_differ_only_by_fixture_generated_anchor() {
        let block_id = BlockId::from_uuid(Uuid::from_u128(1));
        let user_block_id = BlockId::from_uuid(Uuid::from_u128(4));
        let user_authored_id = LogseqUuid::from_uuid(Uuid::from_u128(5));
        let state = ProjectionPageState {
            page: MaterializedPage {
                page_id: PageId::from_uuid(Uuid::from_u128(2)),
                name: crate::oplog::LogicalPageName::parse("Policy").unwrap(),
                path: ManagedPath::parse("pages/policy.md").unwrap(),
                preamble: None,
                blocks: vec![
                    MaterializedBlock {
                        block_id,
                        home_document_id: DocumentId::from_uuid(Uuid::from_u128(3)),
                        parent: None,
                        order: "a".into(),
                        logseq_uuid: None,
                        logseq_identity_origin: None,
                        content: "ordinary content".into(),
                    },
                    MaterializedBlock {
                        block_id: user_block_id,
                        home_document_id: DocumentId::from_uuid(Uuid::from_u128(3)),
                        parent: None,
                        order: "b".into(),
                        logseq_uuid: Some(user_authored_id),
                        logseq_identity_origin: Some(LogseqIdentityOrigin::ExternalImported),
                        content: format!("user-authored\nid:: {user_authored_id}"),
                    },
                ],
                stats: MaterializationStats::default(),
            },
            frontier: FrontierV2::default(),
            claim_evidence: Vec::new(),
        };

        let sparse = render_projection(&state, None).unwrap();
        let dense: Vec<u8> = render_dense_projection_bytes(&state, None).unwrap();
        let expected_instrumentation_id = LogseqUuid::from_uuid(block_id.as_uuid());
        let sparse_document = crate::doc::parse(std::str::from_utf8(&sparse.target).unwrap());
        let sparse_semantics = canonical_semantics(&sparse.target, None);
        let dense_semantics = canonical_semantics(&dense, Some(expected_instrumentation_id));

        assert_eq!(sparse.generated_anchors, []);
        assert!(sparse_document.roots[0]
            .projection()
            .properties
            .iter()
            .all(|(key, _)| !key.eq_ignore_ascii_case("id")));
        assert_eq!(sparse_semantics, dense_semantics);
        assert!(sparse_semantics.roots[1]
            .properties
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("id")
                && value.trim() == user_authored_id.to_string()));
        assert!(std::str::from_utf8(&sparse.target)
            .unwrap()
            .contains("ordinary content"));
        assert!(std::str::from_utf8(&dense)
            .unwrap()
            .contains("ordinary content"));
        assert!(!std::str::from_utf8(&sparse.target)
            .unwrap()
            .contains(&expected_instrumentation_id.to_string()));
        assert!(std::str::from_utf8(&dense)
            .unwrap()
            .contains(&format!("id:: {expected_instrumentation_id}")));
        assert!(std::str::from_utf8(&dense)
            .unwrap()
            .contains(&format!("id:: {user_authored_id}")));
    }
}
