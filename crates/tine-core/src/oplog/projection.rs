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
//!
//! The deterministic projection crash hook remains crate-private:
//!
//! ```compile_fail
//! use tine_core::oplog::projection::fail_next_manifested_projection_during_write_for_harness;
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::io;

use super::{
    AnnotatedIdentity, AnnotatedProjectionBase, BaseBlob, BatchInspection, BlockId, EngineError,
    LogseqIdentityOrigin, LogseqUuid, ManifestProjectionPrecondition, ManifestProjectionTarget,
    ManifestedProjectionIntent, MaterializedBlock, MaterializedPage, ObjectKind, ObjectStore,
    PageId, ProjectionCompletion, ProjectionEndpointId, ProjectionIntent, ProjectionPageState,
    ProjectionPrecondition, ProjectionReceiptStore, ProjectionStoreError,
    ProjectionTombstoneAuthorization, ProjectionWork, ProjectionWorkBlockAuthority,
    ProjectionWorkIndex, ProjectionWorkStatus, ProjectionWorkTarget, ReceiptError,
    ShardedHotEngine, StructuralLocator, StructuralSpan, WorkspaceId,
};
use crate::doc::{DocBlock, Document, SerializeOpts, StructuralLayoutIdentity};
use crate::model::ProjectionRecoveryCleanup;
use crate::oplog::projection_store::MAX_PENDING_PROJECTION_CLEANUP_PER_PASS;
use crate::Graph;

thread_local! {
    // Crate-private deterministic simulator hook at the production manifested
    // projection boundary: intent and attempt authority are durable, but the
    // graph mutation has not started.
    static HARNESS_FAIL_DURING_MANIFESTED_PROJECTION: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    static HARNESS_FAIL_AFTER_FORMATTING_INTENT: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    // Counts only test builds, so the exact-source reuse proof adds no
    // production instrumentation or hot-path work.
    static PAGE_DOCUMENT_BUILD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn page_document_build_count_for_test() -> usize {
    PAGE_DOCUMENT_BUILD_COUNT.with(std::cell::Cell::get)
}

/// Operation-scoped capability for the deterministic manifested-projection
/// fault. Dropping it restores the thread's prior hook state, including during
/// unwinding, so a simulator failure before graph execution cannot fault a
/// later unrelated projection on the same thread.
#[must_use = "the manifested-projection fault must remain scoped to its coordinator operation"]
pub(crate) struct ManifestedProjectionFaultScope {
    previously_armed: bool,
}

impl Drop for ManifestedProjectionFaultScope {
    fn drop(&mut self) {
        HARNESS_FAIL_DURING_MANIFESTED_PROJECTION.with(|fail| fail.set(self.previously_armed));
    }
}

pub(crate) fn fail_next_manifested_projection_during_write_for_harness(
) -> ManifestedProjectionFaultScope {
    let previously_armed =
        HARNESS_FAIL_DURING_MANIFESTED_PROJECTION.with(|fail| fail.replace(true));
    ManifestedProjectionFaultScope { previously_armed }
}

pub(crate) fn fail_next_formatting_adoption_after_intent_for_harness() {
    HARNESS_FAIL_AFTER_FORMATTING_INTENT.with(|fail| fail.set(true));
}

fn fail_during_manifested_projection_for_harness() -> Result<(), ProjectionError> {
    HARNESS_FAIL_DURING_MANIFESTED_PROJECTION.with(|fail| {
        if fail.replace(false) {
            Err(ProjectionError::Work(
                "deterministic failure during manifested projection".into(),
            ))
        } else {
            Ok(())
        }
    })
}

fn fail_after_formatting_intent_for_harness() -> Result<(), ProjectionError> {
    HARNESS_FAIL_AFTER_FORMATTING_INTENT.with(|fail| {
        if fail.replace(false) {
            Err(ProjectionError::Work(
                "deterministic failure after formatting-only intent".into(),
            ))
        } else {
            Ok(())
        }
    })
}

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
    guarded_layout: GuardedProjectionLayout,
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

    fn guarded_layout(&self) -> &GuardedProjectionLayout {
        &self.guarded_layout
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
    base_layout_identities: Vec<StructuralLayoutIdentity>,
    generated_anchors: Vec<PolicyGeneratedAnchor>,
}

/// Identity-bound formatting authority carried from the pure planner (or an
/// authenticated manifested base/target pair) into Graph's singular guarded
/// serializer. Construction stays private to this module so raw target bytes
/// cannot nominate their own identity mapping at the mutation boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GuardedProjectionLayout {
    base: Vec<StructuralLayoutIdentity>,
    target: Vec<StructuralLayoutIdentity>,
}

impl GuardedProjectionLayout {
    fn new(base: Vec<StructuralLayoutIdentity>, target_annotations: &[AnnotatedIdentity]) -> Self {
        Self {
            base,
            target: structural_layout_identities(target_annotations),
        }
    }

    fn from_authenticated_annotations(
        base_annotations: Option<&[AnnotatedIdentity]>,
        target_annotations: &[AnnotatedIdentity],
    ) -> Self {
        Self::new(
            base_annotations.map_or_else(Vec::new, structural_layout_identities),
            target_annotations,
        )
    }

    pub(crate) fn empty() -> Self {
        Self {
            base: Vec::new(),
            target: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn canonical_for_test(document: &Document) -> Self {
        fn collect(
            blocks: &[DocBlock],
            locator: &mut Vec<u32>,
            identities: &mut Vec<StructuralLayoutIdentity>,
        ) {
            for (position, block) in blocks.iter().enumerate() {
                locator.push(u32::try_from(position).expect("test projection tree is too wide"));
                identities.push(StructuralLayoutIdentity {
                    locator: locator.clone(),
                    block_identity: format!("test-projection-{}", identities.len()),
                });
                collect(&block.children, locator, identities);
                locator.pop();
            }
        }

        let mut target = Vec::new();
        collect(&document.roots, &mut Vec::new(), &mut target);
        Self {
            base: Vec::new(),
            target,
        }
    }

    pub(crate) fn base(&self) -> &[StructuralLayoutIdentity] {
        &self.base
    }

    pub(crate) fn target(&self) -> &[StructuralLayoutIdentity] {
        &self.target
    }
}

fn structural_layout_identities(
    annotations: &[AnnotatedIdentity],
) -> Vec<StructuralLayoutIdentity> {
    annotations
        .iter()
        .map(|annotation| StructuralLayoutIdentity {
            locator: annotation.locator().components().to_vec(),
            block_identity: annotation.block_id().to_string(),
        })
        .collect()
}

/// Build exact projection bytes and receipt annotations without touching disk.
pub fn plan_projection(
    workspace_id: WorkspaceId,
    state: &ProjectionPageState,
    expected_base: Option<&[u8]>,
) -> Result<ProjectionPlan, ProjectionError> {
    plan_projection_with_layout_annotations(workspace_id, state, expected_base, None)
}

/// Why an exact external source cannot serve as the projection of accepted
/// semantic state.  These categories are deliberately semantic rather than
/// byte-oriented so bootstrap admission can report the first useful boundary
/// that disagrees.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExactSourceSemanticDifference {
    UnsupportedSourceLayout(String),
    PageKind {
        accepted: &'static str,
        source: &'static str,
    },
    PageName {
        accepted: String,
        source: String,
    },
    PreambleOrPageProperties,
    BlockCount {
        accepted: usize,
        source: usize,
    },
    BlockOrderOrAncestry {
        accepted_locator: Vec<u32>,
        source_locator: Vec<u32>,
    },
    ExplicitBlockIdentity {
        locator: Vec<u32>,
    },
    BlockContent {
        locator: Vec<u32>,
    },
}

impl fmt::Display for ExactSourceSemanticDifference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSourceLayout(detail) => {
                write!(
                    formatter,
                    "source layout is not safely importable: {detail}"
                )
            }
            Self::PageKind { accepted, source } => {
                write!(
                    formatter,
                    "page kind differs: accepted={accepted}, source={source}"
                )
            }
            Self::PageName { accepted, source } => write!(
                formatter,
                "page name differs: accepted={accepted:?}, source={source:?}"
            ),
            Self::PreambleOrPageProperties => {
                formatter.write_str("page preamble or page properties differ")
            }
            Self::BlockCount { accepted, source } => write!(
                formatter,
                "block count differs: accepted={accepted}, source={source}"
            ),
            Self::BlockOrderOrAncestry {
                accepted_locator,
                source_locator,
            } => write!(
                formatter,
                "block order or ancestry differs: accepted locator {accepted_locator:?}, \
                 source locator {source_locator:?}"
            ),
            Self::ExplicitBlockIdentity { locator } => {
                write!(
                    formatter,
                    "explicit block identity differs at locator {locator:?}"
                )
            }
            Self::BlockContent { locator } => {
                write!(formatter, "block content differs at locator {locator:?}")
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum ExactSourceProjectionError {
    Projection(ProjectionError),
    Semantic(ExactSourceSemanticDifference),
}

impl From<ProjectionError> for ExactSourceProjectionError {
    fn from(error: ProjectionError) -> Self {
        Self::Projection(error)
    }
}

/// Prove that `source` is the complete accepted semantic page, then construct
/// an adoption baseline whose target and precondition both name those exact
/// bytes. When ordinary rendering would change harmless trivia, source
/// coordinates come from the parser-owned spans used by external import. When
/// authenticated annotations accompany a Markdown source, they are retained
/// only if ordinary rendering reproduces both the bytes and annotations;
/// otherwise the exact source establishes a guarded parser-owned baseline
/// again. Org keeps its stricter ordinary guarded rendering when authenticated
/// annotations are present.
pub(crate) fn plan_projection_adopting_exact_source(
    workspace_id: WorkspaceId,
    state: &ProjectionPageState,
    source: &[u8],
) -> Result<ProjectionPlan, ExactSourceProjectionError> {
    plan_exact_source_projection(workspace_id, state, source, None)
}

pub(crate) fn plan_projection_with_layout_annotations(
    workspace_id: WorkspaceId,
    state: &ProjectionPageState,
    expected_base: Option<&[u8]>,
    expected_base_annotations: Option<&[AnnotatedIdentity]>,
) -> Result<ProjectionPlan, ProjectionError> {
    let may_adopt_exact_source = expected_base_annotations.is_none()
        || matches!(format_for_page(&state.page)?, ProjectionFormat::Markdown);
    if let Some(source) = expected_base.filter(|_| may_adopt_exact_source) {
        match plan_exact_source_projection(workspace_id, state, source, expected_base_annotations) {
            Ok(plan) => return Ok(plan),
            Err(ExactSourceProjectionError::Semantic(_)) => {}
            Err(ExactSourceProjectionError::Projection(error)) => return Err(error),
        }
    }
    let rendered = render_projection(state, expected_base, expected_base_annotations)?;
    projection_plan_from_rendered(workspace_id, state, expected_base, rendered)
}

fn projection_plan_from_rendered(
    workspace_id: WorkspaceId,
    state: &ProjectionPageState,
    expected_base: Option<&[u8]>,
    rendered: RenderedProjection,
) -> Result<ProjectionPlan, ProjectionError> {
    let base = expected_base.map(|bytes| BaseBlob::new(bytes.to_vec()));
    let precondition = base
        .as_ref()
        .map_or(ProjectionPrecondition::Absent, |base| {
            ProjectionPrecondition::Base(base.description())
        });
    let guarded_layout =
        GuardedProjectionLayout::new(rendered.base_layout_identities, &rendered.annotations);
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
        guarded_layout,
        generated_anchors: rendered.generated_anchors,
    })
}

fn plan_exact_source_projection(
    workspace_id: WorkspaceId,
    state: &ProjectionPageState,
    source: &[u8],
    expected_base_annotations: Option<&[AnnotatedIdentity]>,
) -> Result<ProjectionPlan, ExactSourceProjectionError> {
    let format = format_for_page(&state.page)?;
    let source_text =
        std::str::from_utf8(source).map_err(|_| ProjectionError::InvalidUtf8("projection base"))?;
    let parsed = match format {
        ProjectionFormat::Markdown => crate::doc::try_parse_with_source_spans(source_text),
        ProjectionFormat::Org => crate::org::try_parse_org_with_source_spans(source_text),
    }
    .map_err(|error| {
        ExactSourceProjectionError::Semantic(
            ExactSourceSemanticDifference::UnsupportedSourceLayout(error.to_string()),
        )
    })?;
    let source_is_importable = match format {
        ProjectionFormat::Markdown => {
            crate::doc::markdown_structurally_round_trips_parsed(source_text, &parsed)
        }
        ProjectionFormat::Org => crate::org::org_editable_parsed(source_text, &parsed),
    };
    if !source_is_importable {
        return Err(ExactSourceProjectionError::Semantic(
            ExactSourceSemanticDifference::UnsupportedSourceLayout(match format {
                ProjectionFormat::Markdown => {
                    "Markdown parsing and format-preserving serialization change its semantic document"
                }
                ProjectionFormat::Org => {
                    "Org heading structure does not round-trip through the editable representation"
                }
            }
            .into()),
        ));
    }

    let mut metadata = ProjectionMetadata::with_capacity(state.page.blocks.len());
    let accepted = build_page_document(
        &state.page,
        format,
        ProjectionRenderMode::Sparse,
        Some(&mut metadata),
    )?;
    compare_exact_source_semantics(format, &accepted, &parsed.document)
        .map_err(ExactSourceProjectionError::Semantic)?;
    if parsed.block_spans.len() != metadata.pending_annotations.len() {
        return Err(ExactSourceProjectionError::Semantic(
            ExactSourceSemanticDifference::BlockCount {
                accepted: metadata.pending_annotations.len(),
                source: parsed.block_spans.len(),
            },
        ));
    }

    let annotations = metadata
        .pending_annotations
        .iter()
        .zip(&parsed.block_spans)
        .map(|(pending, span)| {
            Ok(AnnotatedIdentity::new(
                StructuralLocator::new(pending.locator.clone())?,
                StructuralSpan::new(
                    u64::try_from(span.start).map_err(|_| ProjectionError::ProjectionTooLarge)?,
                    u64::try_from(span.end).map_err(|_| ProjectionError::ProjectionTooLarge)?,
                )?,
                pending.block_id,
                pending.logseq_uuid,
            ))
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    metadata
        .generated_anchors
        .sort_unstable_by_key(PolicyGeneratedAnchor::block_id);
    let rendered = render_projection_document(
        format,
        &accepted,
        Some(source_text),
        expected_base_annotations,
        metadata,
    )?;
    let rendered_annotations_match = expected_base_annotations
        .is_none_or(|expected| expected == rendered.annotations.as_slice());
    if rendered.target == source && rendered_annotations_match {
        return projection_plan_from_rendered(workspace_id, state, Some(source), rendered)
            .map_err(ExactSourceProjectionError::Projection);
    }
    let base = BaseBlob::new(source.to_vec());
    let description = base.description();
    let intent = ProjectionIntent::new(
        workspace_id,
        state.page.page_id,
        state.page.path.clone(),
        state.frontier.clone(),
        state.claim_evidence.clone(),
        ProjectionPrecondition::Base(description),
        description,
        annotations.clone(),
    )
    .map_err(ProjectionError::from)?;
    Ok(ProjectionPlan {
        intent,
        base: Some(base),
        target: source.to_vec(),
        guarded_layout: GuardedProjectionLayout::from_authenticated_annotations(
            expected_base_annotations.or(Some(&annotations)),
            &annotations,
        ),
        generated_anchors: rendered.generated_anchors,
    })
}

struct SemanticBlock<'a> {
    locator: Vec<u32>,
    block: &'a DocBlock,
}

fn semantic_blocks(blocks: &[DocBlock]) -> Vec<SemanticBlock<'_>> {
    let mut flattened = Vec::new();
    let mut pending = blocks
        .iter()
        .enumerate()
        .rev()
        .map(|(position, block)| (vec![position as u32], block))
        .collect::<Vec<_>>();
    while let Some((locator, block)) = pending.pop() {
        flattened.push(SemanticBlock {
            locator: locator.clone(),
            block,
        });
        for (position, child) in block.children.iter().enumerate().rev() {
            let mut child_locator = locator.clone();
            child_locator.push(position as u32);
            pending.push((child_locator, child));
        }
    }
    flattened
}

fn explicit_block_ids(block: &DocBlock, format: ProjectionFormat) -> Vec<String> {
    let mut block = block.clone();
    block.is_org = format == ProjectionFormat::Org;
    block
        .properties()
        .into_iter()
        .filter(|(key, _)| crate::doc::property_key_norm(key) == "id")
        .map(|(_, value)| value.trim().to_owned())
        .collect()
}

fn compare_exact_source_semantics(
    format: ProjectionFormat,
    accepted: &Document,
    source: &Document,
) -> Result<(), ExactSourceSemanticDifference> {
    if accepted.pre_block != source.pre_block {
        return Err(ExactSourceSemanticDifference::PreambleOrPageProperties);
    }
    let accepted = semantic_blocks(&accepted.roots);
    let source = semantic_blocks(&source.roots);
    if accepted.len() != source.len() {
        return Err(ExactSourceSemanticDifference::BlockCount {
            accepted: accepted.len(),
            source: source.len(),
        });
    }

    let mut accepted_locations = HashMap::<&str, (usize, &[u32])>::new();
    let mut source_locations = HashMap::<&str, (usize, &[u32])>::new();
    for semantic in &accepted {
        let entry = accepted_locations
            .entry(semantic.block.raw.as_str())
            .or_insert((0, semantic.locator.as_slice()));
        entry.0 += 1;
    }
    for semantic in &source {
        let entry = source_locations
            .entry(semantic.block.raw.as_str())
            .or_insert((0, semantic.locator.as_slice()));
        entry.0 += 1;
    }
    for semantic in &accepted {
        let Some((1, source_locator)) = source_locations.get(semantic.block.raw.as_str()) else {
            continue;
        };
        let Some((1, accepted_locator)) = accepted_locations.get(semantic.block.raw.as_str())
        else {
            continue;
        };
        if accepted_locator != source_locator {
            return Err(ExactSourceSemanticDifference::BlockOrderOrAncestry {
                accepted_locator: accepted_locator.to_vec(),
                source_locator: source_locator.to_vec(),
            });
        }
    }

    for (accepted, source) in accepted.iter().zip(&source) {
        if accepted.locator != source.locator {
            return Err(ExactSourceSemanticDifference::BlockOrderOrAncestry {
                accepted_locator: accepted.locator.clone(),
                source_locator: source.locator.clone(),
            });
        }
        if accepted.block.raw != source.block.raw {
            if explicit_block_ids(accepted.block, format)
                != explicit_block_ids(source.block, format)
            {
                return Err(ExactSourceSemanticDifference::ExplicitBlockIdentity {
                    locator: accepted.locator.clone(),
                });
            }
            return Err(ExactSourceSemanticDifference::BlockContent {
                locator: accepted.locator.clone(),
            });
        }
    }
    Ok(())
}

fn render_projection(
    state: &ProjectionPageState,
    expected_base: Option<&[u8]>,
    expected_base_annotations: Option<&[AnnotatedIdentity]>,
) -> Result<RenderedProjection, ProjectionError> {
    render_projection_page(&state.page, expected_base, expected_base_annotations)
}

fn render_projection_page(
    page: &MaterializedPage,
    expected_base: Option<&[u8]>,
    expected_base_annotations: Option<&[AnnotatedIdentity]>,
) -> Result<RenderedProjection, ProjectionError> {
    let format = format_for_page(page)?;
    let base_text = expected_base
        .map(|bytes| {
            std::str::from_utf8(bytes).map_err(|_| ProjectionError::InvalidUtf8("projection base"))
        })
        .transpose()?;
    let mut metadata = ProjectionMetadata::with_capacity(page.blocks.len());
    let document = build_page_document(
        page,
        format,
        ProjectionRenderMode::Sparse,
        Some(&mut metadata),
    )?;
    render_projection_document(
        format,
        &document,
        base_text,
        expected_base_annotations,
        metadata,
    )
}

/// Render a document and its projection metadata that were built together.
/// Exact-source planning uses this to retain the accepted document it already
/// validated; ordinary callers continue through `render_projection_page`.
fn render_projection_document(
    format: ProjectionFormat,
    document: &Document,
    base_text: Option<&str>,
    expected_base_annotations: Option<&[AnnotatedIdentity]>,
    mut metadata: ProjectionMetadata,
) -> Result<RenderedProjection, ProjectionError> {
    let layout_identities = projection_layout_identities(
        format,
        document,
        base_text,
        expected_base_annotations,
        &metadata.pending_annotations,
    );
    let target = serialize_document(format, document, base_text, &layout_identities).into_bytes();
    let annotations = annotate_serialized_blocks(
        format,
        document,
        base_text,
        &layout_identities,
        &target,
        &metadata.pending_annotations,
    )?;
    metadata
        .generated_anchors
        .sort_unstable_by_key(PolicyGeneratedAnchor::block_id);
    Ok(RenderedProjection {
        target,
        annotations,
        base_layout_identities: layout_identities,
        generated_anchors: metadata.generated_anchors,
    })
}

/// Render a complete editor-requested page through the exact projection
/// serializer without creating an intent or touching the graph directory.
pub(crate) fn render_requested_page_document(
    page: &MaterializedPage,
    expected_base: Option<&[u8]>,
) -> Result<Vec<u8>, ProjectionError> {
    render_projection_page(page, expected_base, None).map(|rendered| rendered.target)
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
    Ok(serialize_document(format, &document, base_text, &[]).into_bytes())
}

fn build_projection_document(
    state: &ProjectionPageState,
    format: ProjectionFormat,
    mode: ProjectionRenderMode,
    mut metadata: Option<&mut ProjectionMetadata>,
) -> Result<Document, ProjectionError> {
    build_page_document(&state.page, format, mode, metadata.as_deref_mut())
}

fn build_page_document(
    page: &MaterializedPage,
    format: ProjectionFormat,
    mode: ProjectionRenderMode,
    mut metadata: Option<&mut ProjectionMetadata>,
) -> Result<Document, ProjectionError> {
    #[cfg(test)]
    PAGE_DOCUMENT_BUILD_COUNT.with(|count| count.set(count.get() + 1));

    let forest = ValidatedForest::new(&page.blocks)?;
    let raw_ids = collect_raw_logseq_ids(&page.blocks, format);
    validate_logseq_state(&page.blocks, &raw_ids)?;

    let mut roots = Vec::with_capacity(forest.roots.len());
    for (root_position, index) in forest.roots.iter().copied().enumerate() {
        roots.push(build_doc_block(
            &page.blocks,
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
        pre_block: page.preamble.clone(),
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

/// Derive and execute a receiver-local projection from one accepted foreign
/// endpoint intent. The foreign target bytes grant no authority: rendering is
/// repeated from accepted semantic state and the receiver's exact current
/// bytes, then committed through the receiver's private receipt store.
pub(crate) fn execute_receiver_local_projection_under_handoff(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    source: &ManifestedProjectionIntent,
    handoff: &crate::model::PublishedHandoffLatch,
    allow_mutation: bool,
) -> Result<Option<bool>, ProjectionError> {
    require_endpoint_authority(graph, receipts, engine)?;
    retire_pending_projection_recovery(graph, receipts)?;
    let endpoint = engine
        .projection_endpoint_binding()
        .ok_or(ProjectionError::EndpointBindingMismatch)?;
    if source.source_endpoint_id() == endpoint.endpoint_id {
        return Err(ProjectionError::ReceiverEndpointIsSource);
    }
    let source_absent = matches!(source.target(), ManifestProjectionTarget::Absent);
    let mut effective_candidate = None;
    let tombstone_authorization = if source_absent {
        match engine.authorize_projection_tombstone(source) {
            Ok(authorization) => Some(authorization),
            Err(EngineError::ProjectionAuthorizationUnavailable) => return Ok(Some(false)),
            Err(error) => return Err(error.into()),
        }
    } else {
        let current_matches_source = match engine.authorize_projection_write(source.page_id()) {
            Ok(current) => {
                current.state().page.path == *source.path()
                    && current.state().frontier == *source.post_frontier()
                    && current.state().claim_evidence == source.claim_evidence()
            }
            Err(EngineError::ProjectionAuthorizationUnavailable) => false,
            Err(error) => return Err(error.into()),
        };
        if !current_matches_source {
            // Most superseded immutable intents are historical evidence only.
            // One exception is an exact-title event selected by the current
            // merged frontier: its original source intent remains the
            // authenticated authority for the winning UTF-8 spelling and for
            // whether that title was explicit or filename-derived.
            match engine.authenticate_effective_title_projection_candidate(source.page_id()) {
                Ok(candidate) => effective_candidate = Some(candidate),
                Err(EngineError::ProjectionAuthorizationUnavailable) => {
                    return Ok(Some(false));
                }
                Err(error) => return Err(error.into()),
            }
        }
        None
    };
    let projection_source = effective_candidate
        .as_ref()
        .map(|candidate| candidate.source())
        .unwrap_or(source);
    let local_base = graph.read_projection_input(projection_source.path())?;
    let mut effective_prior_completion = None;
    let plan = if source_absent {
        receiver_tombstone_plan(
            receipts,
            engine,
            tombstone_authorization
                .as_ref()
                .expect("Absent source has tombstone authorization"),
            local_base.as_deref(),
        )?
    } else if let Some(candidate) = effective_candidate.as_ref() {
        let (completed_intent, completion) =
            receipts.load_completed_receipt(candidate.lifecycle_completion())?;
        let local_base = local_base
            .as_deref()
            .ok_or(ProjectionError::ReceiverBaseMismatch)?;
        let authorization = engine.authorize_effective_title_projection_write(
            candidate,
            projection_source,
            &completed_intent,
            &completion,
            local_base,
        )?;
        let plan = plan_projection(
            engine.workspace_id(),
            authorization.state(),
            Some(local_base),
        )?;
        effective_prior_completion = Some((completed_intent, completion));
        plan
    } else {
        derive_receiver_local_projection(
            engine,
            source,
            endpoint.endpoint_id,
            local_base.as_deref(),
        )?
    };
    receipts.publish_intent(plan.intent(), plan.base().map(BaseBlob::bytes))?;
    let already_complete = receipts.load_completion(plan.intent())?.is_some();
    if !already_complete && !allow_mutation {
        return Ok(None);
    }
    if !already_complete {
        let attempts = receipts.load_attempt_reservations(plan.intent())?;
        let has_attempts = !attempts.is_empty();
        let mut authority = if !has_attempts {
            let reservation = receipts.reserve_attempt(plan.intent())?;
            receipts.begin_mutation(plan.intent(), Some(&reservation))?
        } else {
            receipts.begin_mutation(plan.intent(), None)?
        };
        let target_is_already_exact =
            !source_absent && local_base.as_deref() == Some(plan.target());
        let proof = if target_is_already_exact || has_attempts {
            if source_absent {
                match plan.intent().precondition() {
                    ProjectionPrecondition::Absent => handoff.confirm_removed_page_projection(
                        graph,
                        plan.intent().path().as_str(),
                        &mut authority,
                    )?,
                    ProjectionPrecondition::Base(_) => {
                        let base = plan.base().ok_or(ProjectionError::ReceiverSourceAbsent)?;
                        handoff.recover_removed_page_projection(
                            graph,
                            plan.intent().path().as_str(),
                            base.bytes(),
                            &mut authority,
                        )?
                    }
                }
            } else {
                handoff.recover_page_projection_with_layout(
                    graph,
                    plan.intent().path().as_str(),
                    plan.base().map(BaseBlob::bytes),
                    plan.target(),
                    plan.guarded_layout(),
                    &mut authority,
                )?
            }
        } else if source_absent {
            match local_base.as_deref() {
                Some(base) => handoff.remove_page_projection(
                    graph,
                    plan.intent().path().as_str(),
                    base,
                    &mut authority,
                )?,
                None => handoff.confirm_removed_page_projection(
                    graph,
                    plan.intent().path().as_str(),
                    &mut authority,
                )?,
            }
        } else {
            handoff.write_page_projection_with_layout(
                graph,
                plan.intent().path().as_str(),
                local_base.as_deref(),
                plan.target(),
                plan.guarded_layout(),
                &mut authority,
            )?
        };
        receipts.publish_completion(authority, plan.intent(), &proof)?;
    }
    retire_completed_projection_recovery(graph, receipts, plan.intent())?;
    match tombstone_authorization {
        Some(authorization) => {
            record_completed_tombstone_path(receipts, engine, plan.intent(), authorization)?
        }
        None => {
            if let Some((completed_intent, completion)) = effective_prior_completion.as_ref() {
                let candidate = effective_candidate
                    .as_ref()
                    .expect("effective prior completion has a candidate");
                let exact_local_base = local_base
                    .as_deref()
                    .ok_or(ProjectionError::ReceiverBaseMismatch)?;
                let authorization = engine.authorize_effective_title_projection_write(
                    candidate,
                    projection_source,
                    completed_intent,
                    completion,
                    exact_local_base,
                )?;
                let replay = plan_projection(
                    engine.workspace_id(),
                    authorization.state(),
                    Some(exact_local_base),
                )?;
                if replay.intent() != plan.intent() {
                    return Err(ProjectionError::RecoveryIntentMismatch);
                }
                record_completed_path_target(
                    receipts,
                    engine,
                    source.page_id(),
                    plan.intent(),
                    ProjectionWorkTarget::Present(plan.intent().target()),
                )?;
            } else {
                record_completed_path(receipts, engine, source.page_id(), plan.intent())?;
            }
        }
    }
    Ok(Some(!already_complete))
}

fn receiver_tombstone_plan(
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    authorization: &ProjectionTombstoneAuthorization,
    local_base: Option<&[u8]>,
) -> Result<ProjectionPlan, ProjectionError> {
    let (_, work_index) = engine.enrolled_projection_runtime()?;
    let completed = work_index
        .completed_receipts_for_path(authorization.path())
        .map_err(|error| ProjectionError::Work(error.to_string()))?;
    let mut deletion = completed.iter().filter(|receipt| {
        receipt.page_id() == authorization.page_id()
            && receipt.path() == authorization.path()
            && receipt.frontier() == authorization.frontier()
            && receipt.target() == ProjectionWorkTarget::Absent
    });
    if let Some(receipt) = deletion.next() {
        if deletion.next().is_some() {
            return Err(ProjectionError::ReceiverBaseMismatch);
        }
        let (intent, _) = store.load_completed_receipt(receipt)?;
        let base = store.load_base(&intent)?;
        return Ok(ProjectionPlan {
            intent,
            base,
            target: Vec::new(),
            guarded_layout: GuardedProjectionLayout::empty(),
            generated_anchors: Vec::new(),
        });
    }

    let prior_description = match authorization.prior_frontier() {
        Some(prior_frontier) => {
            let mut prior = completed.iter().filter_map(|receipt| {
                (receipt.page_id() == authorization.page_id()
                    && receipt.path() == authorization.path()
                    && receipt.frontier() == prior_frontier)
                    .then(|| match receipt.target() {
                        ProjectionWorkTarget::Present(description) => Some((receipt, description)),
                        ProjectionWorkTarget::Absent => None,
                    })
                    .flatten()
            });
            match prior.next() {
                Some((receipt, description)) => {
                    if prior.next().is_some() {
                        return Err(ProjectionError::ReceiverBaseMismatch);
                    }
                    store.load_completed_receipt(receipt)?;
                    Some(description)
                }
                None if local_base.is_some() => {
                    return Err(ProjectionError::ReceiverBaseMismatch);
                }
                None => None,
            }
        }
        None => None,
    };
    let make_intent = |precondition| {
        ProjectionIntent::new(
            engine.workspace_id(),
            authorization.page_id(),
            authorization.path().clone(),
            authorization.frontier().clone(),
            Vec::new(),
            precondition,
            super::BlobDescription::of(&[]),
            Vec::new(),
        )
    };
    let (intent, base) = match (prior_description, local_base) {
        (Some(description), Some(bytes)) => {
            if super::BlobDescription::of(bytes) != description {
                return Err(ProjectionError::ReceiverBaseMismatch);
            }
            (
                make_intent(ProjectionPrecondition::Base(description))?,
                Some(BaseBlob::new(bytes.to_vec())),
            )
        }
        (Some(description), None) => {
            let base_intent = make_intent(ProjectionPrecondition::Base(description))?;
            match store.load_intent(base_intent.id()?)? {
                Some(intent) if intent == base_intent => {
                    let base = store
                        .load_base(&intent)?
                        .ok_or(ProjectionError::ReceiverBaseMismatch)?;
                    (intent, Some(base))
                }
                Some(_) => return Err(ProjectionError::ReceiverBaseMismatch),
                None => (make_intent(ProjectionPrecondition::Absent)?, None),
            }
        }
        (None, None) => (make_intent(ProjectionPrecondition::Absent)?, None),
        (None, Some(_)) => return Err(ProjectionError::ReceiverBaseMismatch),
    };
    Ok(ProjectionPlan {
        intent,
        base,
        target: Vec::new(),
        guarded_layout: GuardedProjectionLayout::empty(),
        generated_anchors: Vec::new(),
    })
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
        None,
    )
}

pub(crate) fn execute_manifested_projection_work_under_handoff(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &mut ShardedHotEngine,
    work: &ProjectionWork,
    handoff: &crate::model::PublishedHandoffLatch,
) -> Result<(), ProjectionError> {
    let (archive, work_index) = engine.enrolled_projection_runtime()?;
    execute_manifested_projection_work_with_runtime(
        graph,
        receipts,
        &archive,
        engine,
        &work_index,
        work,
        Some(handoff),
    )
}

fn block_manifested_projection_work(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    work_index: &ProjectionWorkIndex,
    work: &ProjectionWork,
) -> Result<(), ProjectionError> {
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
        .map_err(|error| ProjectionError::Work(error.to_string()))
}

fn execute_manifested_projection_work_with_runtime(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    archive: &ObjectStore,
    engine: &mut ShardedHotEngine,
    work_index: &ProjectionWorkIndex,
    work: &ProjectionWork,
    handoff: Option<&crate::model::PublishedHandoffLatch>,
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
    retire_pending_projection_recovery(graph, receipts)?;
    engine
        .authorize_projection_work(work_index, work)
        .map_err(ProjectionError::Engine)?;
    // A journal drain has no affine process-memory continuation after a
    // restart. Re-entering with the exact already-completed work must therefore
    // authenticate the same archive/intent/receipt authority below and adopt
    // it idempotently. `mark_completed` already performs that exact terminal
    // comparison; blocked, superseded, absent, and merely reserved work remain
    // refusals.
    if !matches!(
        work_index
            .status(work.work_id())
            .map_err(|error| ProjectionError::Work(error.to_string()))?,
        Some(ProjectionWorkStatus::Ready | ProjectionWorkStatus::Completed)
    ) {
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
    let guarded_layout = if target.is_some() {
        GuardedProjectionLayout::from_authenticated_annotations(
            expected_base
                .as_ref()
                .map(AnnotatedProjectionBase::annotations),
            &annotations,
        )
    } else {
        GuardedProjectionLayout::empty()
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
        retire_completed_projection_recovery(graph, receipts, &local_attempt_intent)?;
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
        let result = match (handoff, target) {
            (Some(handoff), Some(target)) => handoff.recover_page_projection_with_layout(
                graph,
                manifested.path().as_str(),
                expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                target,
                &guarded_layout,
                &mut authority,
            ),
            (None, Some(target)) => graph.recover_page_projection_with_layout(
                manifested.path().as_str(),
                expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                target,
                &guarded_layout,
                &mut authority,
            ),
            (Some(handoff), None) => {
                let base = expected_base
                    .as_ref()
                    .ok_or(ProjectionError::WorkIntentMismatch)?;
                handoff.recover_removed_page_projection(
                    graph,
                    manifested.path().as_str(),
                    base.bytes(),
                    &mut authority,
                )
            }
            (None, None) => {
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
        Some((Err(error), _)) if crate::model::is_projection_semantic_refusal(&error) => {
            block_manifested_projection_work(graph, receipts, work_index, work)?;
            return Err(error.into());
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
            fail_during_manifested_projection_for_harness()?;
            let current = graph
                .read_projection_input(work.path())
                .map_err(ProjectionError::Io)?;
            let target_is_already_exact =
                target.is_some_and(|target| current.as_deref() == Some(target));
            let write_result = if target_is_already_exact {
                match (handoff, target.expect("exact present target")) {
                    (Some(handoff), target) => handoff.recover_page_projection_with_layout(
                        graph,
                        manifested.path().as_str(),
                        expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                        target,
                        &guarded_layout,
                        &mut authority,
                    ),
                    (None, target) => graph.recover_page_projection_with_layout(
                        manifested.path().as_str(),
                        expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                        target,
                        &guarded_layout,
                        &mut authority,
                    ),
                }
            } else if target.is_none() && current.is_none() {
                match handoff {
                    Some(handoff) => handoff.confirm_removed_page_projection(
                        graph,
                        manifested.path().as_str(),
                        &mut authority,
                    ),
                    None => graph.confirm_removed_page_projection(
                        manifested.path().as_str(),
                        &mut authority,
                    ),
                }
            } else {
                match (handoff, target) {
                    (Some(handoff), Some(target)) => handoff.write_page_projection_with_layout(
                        graph,
                        manifested.path().as_str(),
                        expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                        target,
                        &guarded_layout,
                        &mut authority,
                    ),
                    (None, Some(target)) => graph.write_page_projection_with_layout(
                        manifested.path().as_str(),
                        expected_base.as_ref().map(AnnotatedProjectionBase::bytes),
                        target,
                        &guarded_layout,
                        &mut authority,
                    ),
                    (Some(handoff), None) => {
                        let base = expected_base
                            .as_ref()
                            .ok_or(ProjectionError::WorkIntentMismatch)?;
                        handoff.remove_page_projection(
                            graph,
                            manifested.path().as_str(),
                            base.bytes(),
                            &mut authority,
                        )
                    }
                    (None, None) => {
                        let base = expected_base
                            .as_ref()
                            .ok_or(ProjectionError::WorkIntentMismatch)?;
                        graph.remove_page_projection(
                            manifested.path().as_str(),
                            base.bytes(),
                            &mut authority,
                        )
                    }
                }
            };
            match write_result {
                Ok(proof) => (proof, authority),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound
                    ) || crate::model::is_projection_semantic_refusal(&error) =>
                {
                    block_manifested_projection_work(graph, receipts, work_index, work)?;
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            }
        }
    };
    receipts.publish_completion(authority, &local_attempt_intent, &proof)?;
    retire_completed_projection_recovery(graph, receipts, &local_attempt_intent)?;
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
    retire_pending_projection_recovery(graph, store)?;
    let authorization = engine.authorize_projection_write(page_id)?;
    let plan = plan_projection(engine.workspace_id(), authorization.state(), expected_base)?;
    store.publish_intent(plan.intent(), plan.base().map(BaseBlob::bytes))?;
    let reservation = store.reserve_attempt(plan.intent())?;
    let mut authority = store.begin_mutation(plan.intent(), Some(&reservation))?;
    let proof = graph.write_page_projection_with_layout(
        plan.intent().path().as_str(),
        expected_base,
        plan.target(),
        plan.guarded_layout(),
        &mut authority,
    )?;
    let completion = store.publish_completion(authority, plan.intent(), &proof)?;
    retire_completed_projection_recovery(graph, store, plan.intent())?;
    record_completed_path(store, engine, page_id, plan.intent())?;
    debug_assert_eq!(authorization.state().page.page_id, page_id);
    Ok(ProjectionWrite { plan, completion })
}

/// Adopt an exact, semantically unchanged external representation as this
/// endpoint's next projection/import baseline. This publishes only a
/// device-local intent/base/completion and completed-path point; it does not
/// author an operation or an operation batch.
pub(crate) fn adopt_existing_projection_formatting(
    graph: &Graph,
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    handoff: &crate::model::HandoffSafeGuard,
    page_id: PageId,
    observed_bytes: &[u8],
    observed_annotations: &[AnnotatedIdentity],
) -> Result<(), ProjectionError> {
    require_endpoint_authority(graph, store, engine)?;
    let authorization = engine.authorize_projection_write(page_id)?;
    let current = graph
        .read_projection_input(&authorization.state().page.path)?
        .ok_or_else(|| ProjectionError::Work("formatting-only source disappeared".into()))?;
    if current != observed_bytes {
        return Err(ProjectionError::Work(
            "formatting-only source changed after import observation".into(),
        ));
    }
    let plan = plan_projection_with_layout_annotations(
        engine.workspace_id(),
        authorization.state(),
        Some(observed_bytes),
        Some(observed_annotations),
    )?;
    if plan.target() != observed_bytes {
        return Err(ProjectionError::Work(
            "formatting-only source is not the exact accepted semantic state".into(),
        ));
    }
    store.publish_intent(plan.intent(), plan.base().map(BaseBlob::bytes))?;
    fail_after_formatting_intent_for_harness()?;
    if store.load_completion(plan.intent())?.is_none() {
        let attempts = store.load_attempt_reservations(plan.intent())?;
        let mut authority = if attempts.is_empty() {
            let reservation = store.reserve_attempt(plan.intent())?;
            store.begin_mutation(plan.intent(), Some(&reservation))?
        } else {
            store.begin_mutation(plan.intent(), None)?
        };
        let proof = handoff.confirm_existing_page_projection(
            graph,
            plan.intent().path().as_str(),
            plan.target(),
            &mut authority,
        )?;
        store.publish_completion(authority, plan.intent(), &proof)?;
    }
    record_adopted_formatting_path(store, engine, page_id, plan.intent())
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
    retire_pending_projection_recovery(graph, store)?;
    for intent in store.incomplete_intents()? {
        let authorization = engine.authorize_projection_recovery(
            intent.page_id(),
            intent.frontier(),
            intent.claim_evidence(),
        )?;
        let base = store.load_base(&intent)?;
        let expected_base = base.as_ref().map(BaseBlob::bytes);
        let formatting_adoption =
            expected_base.is_some_and(|bytes| super::BlobDescription::of(bytes) == intent.target());
        let plan = if formatting_adoption {
            plan_projection_with_layout_annotations(
                engine.workspace_id(),
                authorization.state(),
                expected_base,
                Some(intent.annotations()),
            )?
        } else {
            plan_projection(engine.workspace_id(), authorization.state(), expected_base)?
        };
        if plan.intent() != &intent {
            return Err(ProjectionError::RecoveryIntentMismatch);
        }
        let attempts = store.load_attempt_reservations(&intent)?;
        let recovery_attempt = if attempts.is_empty() {
            None
        } else {
            let mut authority = store.begin_mutation(&intent, None)?;
            let result = graph.recover_page_projection_with_layout(
                intent.path().as_str(),
                expected_base,
                plan.target(),
                plan.guarded_layout(),
                &mut authority,
            );
            Some((result, authority))
        };
        let (proof, authority) = match recovery_attempt {
            Some((Ok(proof), authority)) => (proof, authority),
            None => {
                let mut recovery_authority = store.begin_mutation(&intent, None)?;
                match graph.recover_page_projection_with_layout(
                    intent.path().as_str(),
                    expected_base,
                    plan.target(),
                    plan.guarded_layout(),
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
                        let proof = graph.write_page_projection_with_layout(
                            intent.path().as_str(),
                            expected_base,
                            plan.target(),
                            plan.guarded_layout(),
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
                let proof = graph.write_page_projection_with_layout(
                    intent.path().as_str(),
                    expected_base,
                    plan.target(),
                    plan.guarded_layout(),
                    &mut authority,
                )?;
                (proof, authority)
            }
            Some((Err(error), _)) => return Err(error.into()),
        };
        let completion = store.reconstruct_completion(authority, &intent, plan.target(), &proof)?;
        retire_completed_projection_recovery(graph, store, &intent)?;
        // Historical recovery remains compatible and preserves its durable
        // receipt, but a completion that is no longer the current accepted
        // page state must not replace point-addressable import authority.
        let recorded = if formatting_adoption {
            record_adopted_formatting_path(store, engine, intent.page_id(), &intent)
        } else {
            record_completed_path(store, engine, intent.page_id(), &intent)
        };
        match recorded {
            Ok(()) | Err(ProjectionError::RecoveryIntentMismatch) => {}
            Err(error) => return Err(error),
        }
        debug_assert_eq!(authorization.state().page.page_id, intent.page_id());
        recovered.push(ProjectionWrite { plan, completion });
    }
    Ok(recovered)
}

fn retire_completed_projection_recovery(
    graph: &Graph,
    store: &ProjectionReceiptStore,
    intent: &ProjectionIntent,
) -> Result<(), ProjectionError> {
    let intent_id = intent.id()?;
    for (pending_intent, record) in
        store.pending_projection_cleanup_bounded(MAX_PENDING_PROJECTION_CLEANUP_PER_PASS)?
    {
        if pending_intent.id()? != intent_id {
            continue;
        }
        if store.load_completion(&pending_intent)?.is_none() {
            continue;
        }
        retire_one_projection_recovery(graph, store, &pending_intent, &record)?;
    }
    Ok(())
}

pub(super) fn retire_pending_projection_recovery(
    graph: &Graph,
    store: &ProjectionReceiptStore,
) -> Result<(), ProjectionError> {
    for (intent, record) in
        store.pending_projection_cleanup_bounded(MAX_PENDING_PROJECTION_CLEANUP_PER_PASS)?
    {
        if store.load_completion(&intent)?.is_none() {
            continue;
        }
        retire_one_projection_recovery(graph, store, &intent, &record)?;
    }
    Ok(())
}

pub(super) fn retire_one_projection_recovery(
    graph: &Graph,
    store: &ProjectionReceiptStore,
    intent: &ProjectionIntent,
    record: &super::LocalProjectionEvidenceRecord,
) -> Result<(), ProjectionError> {
    let observation = graph.retire_completed_projection_recovery(
        intent.path().as_str(),
        std::slice::from_ref(record),
        None,
    )?;
    match observation {
        ProjectionRecoveryCleanup::Missing
        | ProjectionRecoveryCleanup::Retired
        | ProjectionRecoveryCleanup::ConflictRetained { .. } => {
            store.retire_pending_projection_cleanup(record)?;
        }
        ProjectionRecoveryCleanup::Quarantined => {
            let Some(retirement) = store.projection_cleanup_grace_elapsed(record)? else {
                return Ok(());
            };
            match graph.retire_completed_projection_recovery(
                intent.path().as_str(),
                std::slice::from_ref(record),
                Some(&retirement),
            )? {
                ProjectionRecoveryCleanup::Missing
                | ProjectionRecoveryCleanup::Retired
                | ProjectionRecoveryCleanup::ConflictRetained { .. } => {
                    store.retire_pending_projection_cleanup(record)?;
                }
                ProjectionRecoveryCleanup::Quarantined => {
                    store.reset_projection_cleanup_grace(record)?;
                }
            }
        }
    }
    Ok(())
}

/// Make every durable completion visible through the enrolled authenticated
/// completed-path tree.  Both ordinary writes and crash recovery use this
/// route; otherwise a recovered receipt would be durable but intentionally
/// invisible to bounded external import.
fn record_completed_path(
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    page_id: PageId,
    intent: &ProjectionIntent,
) -> Result<(), ProjectionError> {
    record_completed_path_with_authorization(store, engine, page_id, intent)
}

fn record_adopted_formatting_path(
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    page_id: PageId,
    intent: &ProjectionIntent,
) -> Result<(), ProjectionError> {
    let current = engine.authorize_projection_write(page_id)?;
    if current.state().page.path != *intent.path()
        || current.state().frontier != *intent.frontier()
        || current.state().claim_evidence != intent.claim_evidence()
    {
        return Err(ProjectionError::RecoveryIntentMismatch);
    }
    let base = store.load_base(intent)?;
    let Some(base) = base.as_ref() else {
        return Err(ProjectionError::RecoveryIntentMismatch);
    };
    if base.description() != intent.target() {
        return Err(ProjectionError::RecoveryIntentMismatch);
    }
    let replay = plan_projection_with_layout_annotations(
        engine.workspace_id(),
        current.state(),
        Some(base.bytes()),
        Some(intent.annotations()),
    )?;
    if replay.intent() != intent {
        return Err(ProjectionError::RecoveryIntentMismatch);
    }
    let (_, work_index) = engine.enrolled_projection_runtime()?;
    let prior = work_index
        .completed_receipts_for_path(intent.path())
        .map_err(|error| ProjectionError::Work(error.to_string()))?;
    let [prior] = prior.as_slice() else {
        return Err(ProjectionError::Work(
            "formatting adoption requires one exact prior completed-path authority".into(),
        ));
    };
    let (engine_history_generation, engine_history_root) =
        engine.projection_completion_history_authority()?;
    let authority = store.completed_direct_authority(
        intent,
        ProjectionWorkTarget::Present(intent.target()),
        engine_history_generation,
        engine_history_root,
    )?;
    work_index
        .mark_formatting_adopted(authority, prior)
        .map_err(|error| ProjectionError::Work(error.to_string()))?;
    Ok(())
}

fn record_completed_tombstone_path(
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    intent: &ProjectionIntent,
    authorization: ProjectionTombstoneAuthorization,
) -> Result<(), ProjectionError> {
    if authorization.page_id() != intent.page_id()
        || authorization.path() != intent.path()
        || authorization.frontier() != intent.frontier()
        || !intent.claim_evidence().is_empty()
        || intent.target() != super::BlobDescription::of(&[])
        || !intent.annotations().is_empty()
    {
        return Err(ProjectionError::RecoveryIntentMismatch);
    }
    record_completed_path_target(
        store,
        engine,
        intent.page_id(),
        intent,
        ProjectionWorkTarget::Absent,
    )
}

fn record_completed_path_with_authorization(
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    page_id: PageId,
    intent: &ProjectionIntent,
) -> Result<(), ProjectionError> {
    // Revalidate the completed intent against the current accepted page before
    // exposing it as point-addressable authority. Historical recovery is
    // allowed to inspect an old frontier, but it must never replace the
    // authority for a newer accepted frontier or a reused path.
    let current = engine.authorize_projection_write(page_id)?;
    if current.state().page.path != *intent.path()
        || current.state().frontier != *intent.frontier()
        || current.state().claim_evidence != intent.claim_evidence()
    {
        return Err(ProjectionError::RecoveryIntentMismatch);
    }
    let base = store.load_base(intent)?;
    let base_bytes = base.as_ref().map(BaseBlob::bytes);
    let replay =
        if base_bytes.is_some_and(|bytes| intent.target() == super::BlobDescription::of(bytes)) {
            plan_projection_with_layout_annotations(
                engine.workspace_id(),
                current.state(),
                base_bytes,
                Some(intent.annotations()),
            )?
        } else {
            plan_projection(engine.workspace_id(), current.state(), base_bytes)?
        };
    if replay.intent() != intent {
        return Err(ProjectionError::RecoveryIntentMismatch);
    }

    record_completed_path_target(
        store,
        engine,
        page_id,
        intent,
        ProjectionWorkTarget::Present(intent.target()),
    )
}

fn record_completed_path_target(
    store: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    page_id: PageId,
    intent: &ProjectionIntent,
    target: ProjectionWorkTarget,
) -> Result<(), ProjectionError> {
    // The compatibility writer still produces a normal enrolled completion.
    // Mirror it into the authenticated completed-path tree when its accepted
    // work row is present, so sparse import never has to fall back to receipt
    // directory enumeration for this path.
    if let Ok((_, work_index)) = engine.enrolled_projection_runtime() {
        let mut exact = work_index
            .pending_for_path(intent.path())
            .map_err(|error| ProjectionError::Work(error.to_string()))?
            .into_iter()
            .filter(|work| {
                work.page_id() == page_id
                    && work.post_frontier() == intent.frontier()
                    && work.target() == target
            });
        if let Some(work) = exact.next() {
            if exact.next().is_some() {
                return Err(ProjectionError::Work(
                    "multiple ready work rows match one direct projection completion".into(),
                ));
            }
            let authority = store.completed_work_authority(&work, intent)?;
            work_index
                .mark_completed(authority)
                .map_err(|error| ProjectionError::Work(error.to_string()))?;
        } else {
            let (engine_history_generation, engine_history_root) =
                engine.projection_completion_history_authority()?;
            let authority = store.completed_direct_authority(
                intent,
                target,
                engine_history_generation,
                engine_history_root,
            )?;
            work_index
                .mark_direct_completed(authority)
                .map_err(|error| ProjectionError::Work(error.to_string()))?;
        }
    }
    Ok(())
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
    if page.path.is_markdown() {
        Ok(ProjectionFormat::Markdown)
    } else if page.path.is_org() {
        Ok(ProjectionFormat::Org)
    } else {
        Err(ProjectionError::UnsupportedFormat(
            page.path.as_str().into(),
        ))
    }
}

fn u32_index(value: usize) -> Result<u32, ProjectionError> {
    u32::try_from(value).map_err(|_| ProjectionError::TreeTooWide)
}

fn projection_layout_identities(
    format: ProjectionFormat,
    document: &Document,
    base: Option<&str>,
    base_annotations: Option<&[AnnotatedIdentity]>,
    target_annotations: &[PendingAnnotation],
) -> Vec<StructuralLayoutIdentity> {
    if let Some(annotations) = base_annotations {
        return annotations
            .iter()
            .map(|annotation| StructuralLayoutIdentity {
                locator: annotation.locator().components().to_vec(),
                block_identity: annotation.block_id().to_string(),
            })
            .collect();
    }
    let source_is_exact_semantic_document = base.is_some_and(|source| {
        let parsed = match format {
            ProjectionFormat::Markdown => crate::doc::parse(source),
            ProjectionFormat::Org => crate::org::parse_org(source),
        };
        parsed == *document
    });
    if !source_is_exact_semantic_document {
        return Vec::new();
    }
    target_annotations
        .iter()
        .map(|annotation| StructuralLayoutIdentity {
            locator: annotation.locator.clone(),
            block_identity: annotation.block_id.to_string(),
        })
        .collect()
}

fn serialize_document(
    format: ProjectionFormat,
    document: &Document,
    base: Option<&str>,
    layout_identities: &[StructuralLayoutIdentity],
) -> String {
    match format {
        ProjectionFormat::Markdown => {
            let serialized = crate::doc::serialize_with(
                document,
                &SerializeOpts::detect_with_layout_identities(base, layout_identities),
            );
            if base.is_some_and(|text| text.contains("\r\n")) {
                serialized.replace('\n', "\r\n")
            } else {
                serialized
            }
        }
        ProjectionFormat::Org => crate::org::serialize_org_detect_with_layout_identities(
            document,
            base,
            layout_identities,
        ),
    }
}

fn annotate_serialized_blocks(
    format: ProjectionFormat,
    document: &Document,
    base: Option<&str>,
    layout_identities: &[StructuralLayoutIdentity],
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
        format,
        &mut marked.roots,
        pending,
        &marker_prefix,
        &mut marked_count,
    )?;
    if marked_count != pending.len() {
        return Err(ProjectionError::SpanInstrumentationMismatch);
    }
    let marked_bytes = serialize_document(format, &marked, base, layout_identities).into_bytes();

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
    format: ProjectionFormat,
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
        let start_offset = match format {
            ProjectionFormat::Markdown => {
                crate::outline::markdown_unbulleted_heading_line_end(&block.raw).unwrap_or(0)
            }
            ProjectionFormat::Org => 0,
        };
        block.raw = format!(
            "{}{start}{}{end}",
            &block.raw[..start_offset],
            &block.raw[start_offset..]
        );
        *index += 1;
        mark_document_blocks(format, &mut block.children, pending, marker_prefix, index)?;
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
    ReceiverBaseMismatch,
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
            Self::ReceiverBaseMismatch => f.write_str(
                "receiver deletion target lacks exact prior completed projection authority",
            ),
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
    use crate::oplog::{
        BlobDescription, CrdtPeerCounter, CrdtPeerId, DocumentDependencies, DocumentId, FrontierV2,
        ManagedPath, MaterializationStats, ProjectionClaimEvidence, ProjectionClaimParticipant,
    };

    #[derive(Debug, Eq, PartialEq)]
    struct CanonicalDocument {
        preamble: Option<String>,
        roots: Vec<CanonicalBlock>,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct CanonicalBlock {
        visible: String,
        properties: Vec<(String, String)>,
        marker: Option<String>,
        tags: Vec<String>,
        page_refs: Vec<String>,
        block_refs: Vec<String>,
        scheduled: Option<String>,
        deadline: Option<String>,
        children: Vec<CanonicalBlock>,
    }

    #[derive(Debug)]
    struct DenseOnlyAnchor {
        locator: Vec<usize>,
        logseq_uuid: LogseqUuid,
    }

    struct DenseSparseCorpusCase {
        name: &'static str,
        state: ProjectionPageState,
        base: Option<&'static [u8]>,
        expected: CanonicalDocument,
        dense_only_anchors: Vec<DenseOnlyAnchor>,
        sparse_generated_anchors: Vec<(BlockId, LogseqUuid)>,
        ordinary_idless_locators: Vec<Vec<usize>>,
        user_authored_byte_fragments: Vec<String>,
        expect_crlf: bool,
    }

    fn canonical_semantics(
        format: ProjectionFormat,
        bytes: &[u8],
        dense_only_anchors: &[DenseOnlyAnchor],
    ) -> CanonicalDocument {
        let text = std::str::from_utf8(bytes).unwrap();
        let document = match format {
            ProjectionFormat::Markdown => crate::doc::parse(text),
            ProjectionFormat::Org => crate::org::parse_org(text),
        };
        CanonicalDocument {
            preamble: document.pre_block,
            roots: canonical_blocks(&document.roots, &mut Vec::new(), dense_only_anchors),
        }
    }

    fn canonical_blocks(
        blocks: &[crate::doc::DocBlock],
        locator: &mut Vec<usize>,
        dense_only_anchors: &[DenseOnlyAnchor],
    ) -> Vec<CanonicalBlock> {
        blocks
            .iter()
            .enumerate()
            .map(|(position, block)| {
                locator.push(position);
                let dense_only_uuid = dense_only_anchors
                    .iter()
                    .find(|anchor| anchor.locator == *locator)
                    .map(|anchor| anchor.logseq_uuid.to_string());
                let projection = block.projection();
                let canonical = CanonicalBlock {
                    visible: projection.visible.clone(),
                    properties: projection
                        .properties
                        .iter()
                        .filter(|(key, value)| {
                            !key.eq_ignore_ascii_case("id")
                                || dense_only_uuid.as_deref() != Some(value.trim())
                        })
                        .cloned()
                        .collect(),
                    marker: projection.marker.clone(),
                    tags: projection.tags.clone(),
                    page_refs: projection.refs_page.clone(),
                    block_refs: projection.block_refs.clone(),
                    scheduled: projection.scheduled.clone(),
                    deadline: projection.deadline.clone(),
                    children: canonical_blocks(&block.children, locator, dense_only_anchors),
                };
                locator.pop();
                canonical
            })
            .collect()
    }

    fn expected_block(
        visible: &str,
        properties: &[(&str, &str)],
        marker: Option<&str>,
        tags: &[&str],
        page_refs: &[&str],
        block_refs: &[&str],
        scheduled: Option<&str>,
        deadline: Option<&str>,
        children: Vec<CanonicalBlock>,
    ) -> CanonicalBlock {
        CanonicalBlock {
            visible: visible.into(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
            marker: marker.map(Into::into),
            tags: tags.iter().map(|tag| (*tag).into()).collect(),
            page_refs: page_refs.iter().map(|page| (*page).into()).collect(),
            block_refs: block_refs.iter().map(|block| (*block).into()).collect(),
            scheduled: scheduled.map(Into::into),
            deadline: deadline.map(Into::into),
            children,
        }
    }

    fn block_at<'a>(roots: &'a [CanonicalBlock], locator: &[usize]) -> &'a CanonicalBlock {
        let (first, rest) = locator.split_first().expect("corpus locator is non-empty");
        let block = &roots[*first];
        if rest.is_empty() {
            block
        } else {
            block_at(&block.children, rest)
        }
    }

    fn has_id(block: &CanonicalBlock, uuid: LogseqUuid) -> bool {
        block
            .properties
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("id") && value.trim() == uuid.to_string())
    }

    fn assert_expected_line_endings(name: &str, bytes: &[u8], expect_crlf: bool) {
        if expect_crlf {
            assert!(
                bytes.iter().enumerate().all(|(index, byte)| {
                    *byte != b'\n' || index > 0 && bytes[index - 1] == b'\r'
                }),
                "{name} must retain CRLF line endings"
            );
        } else {
            assert!(
                !bytes.contains(&b'\r'),
                "{name} must use canonical LF line endings"
            );
        }
    }

    fn structural_layout_state(
        path: &str,
        blocks: Vec<(u128, Option<u128>, &str, String, Option<LogseqUuid>)>,
    ) -> ProjectionPageState {
        let home_document_id = DocumentId::from_uuid(Uuid::from_u128(80_000));
        let blocks = blocks
            .into_iter()
            .map(
                |(id, parent, order, content, logseq_uuid)| MaterializedBlock {
                    block_id: BlockId::from_uuid(Uuid::from_u128(id)),
                    home_document_id,
                    parent: parent.map(|id| BlockId::from_uuid(Uuid::from_u128(id))),
                    order: order.into(),
                    logseq_uuid,
                    logseq_identity_origin: logseq_uuid
                        .map(|_| LogseqIdentityOrigin::ExternalImported),
                    content,
                },
            )
            .collect::<Vec<_>>();
        let claim_evidence = blocks
            .iter()
            .filter_map(|block| {
                block.logseq_uuid.map(|logseq_uuid| {
                    ProjectionClaimEvidence::new(
                        logseq_uuid,
                        vec![ProjectionClaimParticipant::new(
                            block.block_id,
                            block.home_document_id,
                        )],
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let frontier = if claim_evidence.is_empty() {
            FrontierV2::default()
        } else {
            FrontierV2::new(vec![DocumentDependencies::new(
                home_document_id,
                vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(80_003), 0)],
                Vec::new(),
            )
            .unwrap()])
            .unwrap()
        };
        ProjectionPageState {
            page: MaterializedPage {
                page_id: PageId::from_uuid(Uuid::from_u128(80_001)),
                home_document_id,
                name: crate::oplog::LogicalPageName::parse("Structural Layout").unwrap(),
                path: ManagedPath::parse(path).unwrap(),
                kind: crate::oplog::ManagedTextKind::Page,
                preamble: None,
                blocks,
                stats: MaterializationStats::default(),
            },
            frontier,
            claim_evidence,
        }
    }

    fn reproject_with_source_identities(
        base_state: &ProjectionPageState,
        source: &str,
        target_state: &ProjectionPageState,
    ) -> ProjectionPlan {
        let base = plan_projection(
            WorkspaceId::from_uuid(Uuid::from_u128(80_002)),
            base_state,
            Some(source.as_bytes()),
        )
        .unwrap();
        assert_eq!(base.target(), source.as_bytes());
        plan_projection_with_layout_annotations(
            WorkspaceId::from_uuid(Uuid::from_u128(80_002)),
            target_state,
            Some(source.as_bytes()),
            Some(base.intent().annotations()),
        )
        .unwrap()
    }

    #[test]
    fn exact_source_adoption_preserves_equivalent_layout_and_source_spans() {
        let mut state = structural_layout_state(
            "pages/layout.md",
            vec![
                (80_011, None, "a", "alpha".into(), None),
                (
                    80_012,
                    Some(80_011),
                    "a",
                    "child\n\ncontinuation".into(),
                    None,
                ),
                (80_013, None, "b", "omega".into(), None),
            ],
        );
        state.page.preamble = Some("title:: Structural Layout".into());
        let source = concat!(
            "title:: Structural Layout\r\n",
            "\r\n",
            "- alpha\r\n",
            "\r\n",
            "\t- child\r\n",
            "\t  \r\n",
            "\t  continuation\r\n",
            " \t\r\n",
            "- omega"
        )
        .as_bytes();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(80_002));

        let document_builds_before = page_document_build_count_for_test();
        let adopted = plan_projection_adopting_exact_source(workspace, &state, source).unwrap();
        assert_eq!(
            page_document_build_count_for_test(),
            document_builds_before + 1,
            "exact-source planning must build its accepted document once before rendering the shadow source"
        );
        assert_eq!(adopted.target(), source);
        assert_eq!(adopted.intent().target(), BlobDescription::of(source));
        assert_eq!(
            adopted.intent().precondition(),
            &ProjectionPrecondition::Base(BlobDescription::of(source))
        );
        assert_eq!(adopted.intent().annotations().len(), 3);
        for annotation in adopted.intent().annotations() {
            let span = annotation.span();
            let owned = &source[span.start() as usize..span.end() as usize];
            assert!(owned.starts_with(b"- ") || owned.starts_with(b"\t- "));
        }
        let serialized = render_projection(&state, Some(source), None).unwrap();
        assert_ne!(
            serialized.target, source,
            "whitespace-bearing inter-block trivia must exercise semantic adoption rather than byte-identical rendering"
        );
        let canonical_source = String::from_utf8(source.to_vec())
            .unwrap()
            .replace(" \t\r\n- omega", "\r\n- omega")
            .into_bytes();
        let canonical_rendered = render_projection(&state, Some(&canonical_source), None).unwrap();
        assert_eq!(canonical_rendered.target, canonical_source);
        let canonical_adopted =
            plan_projection_adopting_exact_source(workspace, &state, &canonical_source).unwrap();
        assert_eq!(
            canonical_rendered.annotations,
            canonical_adopted.intent().annotations(),
            "byte-equal sources must remain compatible with canonical projector receipts"
        );

        let replay = plan_projection(workspace, &state, Some(source)).unwrap();
        assert_eq!(replay.intent(), adopted.intent());
        assert_eq!(replay.target(), source);

        let mut edited = state.clone();
        edited.page.blocks[2].content = "omega edited".into();
        let next = plan_projection_with_layout_annotations(
            workspace,
            &edited,
            Some(source),
            Some(adopted.intent().annotations()),
        )
        .unwrap();
        assert_ne!(next.target(), source);
        assert_eq!(
            next.intent().precondition(),
            &ProjectionPrecondition::Base(BlobDescription::of(source))
        );
    }

    #[test]
    fn authenticated_exact_source_adoption_retains_markdown_whitespace_layouts() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(80_004));
        let cases = [
            (
                "empty-root-bullet",
                "pages/empty-root.md",
                vec![(80_111, None, "a", String::new(), None)],
                "- \n",
            ),
            (
                "empty-nested-bullet",
                "pages/empty-nested.md",
                vec![
                    (80_121, None, "a", "parent".into(), None),
                    (80_122, Some(80_121), "a", String::new(), None),
                ],
                "- parent\n  - \n",
            ),
            (
                "empty-bullet-crlf",
                "pages/empty-crlf.md",
                vec![(80_131, None, "a", String::new(), None)],
                "- \r\n",
            ),
            (
                "nonempty-trailing-space",
                "pages/nonempty-trailing.md",
                vec![(80_141, None, "a", "keeps trailing ".into(), None)],
                "- keeps trailing \n",
            ),
        ];

        for (name, path, blocks, source) in cases {
            let state = structural_layout_state(path, blocks);
            let imported =
                plan_projection_adopting_exact_source(workspace, &state, source.as_bytes())
                    .unwrap_or_else(|error| panic!("{name} exact-source import failed: {error:?}"));
            let replay = plan_projection_with_layout_annotations(
                workspace,
                &state,
                Some(source.as_bytes()),
                Some(imported.intent().annotations()),
            )
            .unwrap_or_else(|error| panic!("{name} authenticated replay failed: {error}"));
            assert_eq!(
                replay.target(),
                source.as_bytes(),
                "{name} changed source bytes"
            );
            assert_eq!(
                replay.intent().annotations(),
                imported.intent().annotations(),
                "{name} changed source annotations"
            );
        }
    }

    #[test]
    fn unannotated_exact_source_adoption_remains_available_for_org() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(80_005));
        let state = structural_layout_state(
            "journals/2026_08_05.org",
            vec![(80_151, None, "a", "headline".into(), None)],
        );
        let source = b"* headline\r\n";
        let plan = plan_projection_with_layout_annotations(workspace, &state, Some(source), None)
            .expect("unannotated exact Org source must remain adoptable");
        assert_eq!(plan.target(), source);
    }

    #[test]
    fn exact_source_adoption_rejects_each_relevant_semantic_difference() {
        let explicit = LogseqUuid::from_uuid(Uuid::from_u128(80_020));
        let mut state = structural_layout_state(
            "pages/layout.md",
            vec![
                (80_011, None, "a", "alpha".into(), None),
                (80_012, Some(80_011), "a", "bravo".into(), None),
                (
                    80_013,
                    None,
                    "b",
                    format!("omega\nid:: {explicit}"),
                    Some(explicit),
                ),
            ],
        );
        state.page.preamble = Some("title:: Structural Layout\nstatus:: accepted".into());
        let different_explicit = LogseqUuid::from_uuid(Uuid::from_u128(80_021));
        let cases = [
            (
                "dropped block",
                "title:: Structural Layout\nstatus:: accepted\n\n- alpha\n\t- bravo\n",
                "block count differs",
            ),
            (
                "changed content",
                &format!(
                    "title:: Structural Layout\nstatus:: accepted\n\n- alpha changed\n\t- bravo\n- omega\n  id:: {explicit}\n"
                ),
                "block content differs",
            ),
            (
                "changed ancestry",
                &format!(
                    "title:: Structural Layout\nstatus:: accepted\n\n- alpha\n- bravo\n- omega\n  id:: {explicit}\n"
                ),
                "block order or ancestry differs",
            ),
            (
                "changed order",
                &format!(
                    "title:: Structural Layout\nstatus:: accepted\n\n- omega\n  id:: {explicit}\n- alpha\n\t- bravo\n"
                ),
                "block order or ancestry differs",
            ),
            (
                "changed page property",
                &format!(
                    "title:: Structural Layout\nstatus:: changed\n\n- alpha\n\t- bravo\n- omega\n  id:: {explicit}\n"
                ),
                "page preamble or page properties differ",
            ),
            (
                "changed explicit identity",
                &format!(
                    "title:: Structural Layout\nstatus:: accepted\n\n- alpha\n\t- bravo\n- omega\n  id:: {different_explicit}\n"
                ),
                "explicit block identity differs",
            ),
        ];

        for (name, source, expected) in cases {
            let error = plan_projection_adopting_exact_source(
                WorkspaceId::from_uuid(Uuid::from_u128(80_002)),
                &state,
                source.as_bytes(),
            )
            .unwrap_err();
            let ExactSourceProjectionError::Semantic(difference) = error else {
                panic!("{name} produced a non-semantic error: {error:?}");
            };
            assert!(
                difference.to_string().contains(expected),
                "{name}: {difference}"
            );
        }
    }

    #[test]
    fn structural_trivia_follows_only_receipt_bound_identities_across_nested_edits() {
        let anchored = LogseqUuid::from_uuid(Uuid::from_u128(80_010));
        let base = structural_layout_state(
            "pages/layout.md",
            vec![
                (80_011, None, "a", "alpha".into(), None),
                (80_012, Some(80_011), "a", "child one".into(), None),
                (80_013, Some(80_011), "b", "child two".into(), None),
                (
                    80_014,
                    None,
                    "b",
                    format!("omega\nid:: {anchored}"),
                    Some(anchored),
                ),
            ],
        );
        let source =
            format!("- alpha\n\t- child one\n\n\t- child two\n\n- omega\n  id:: {anchored}\n");

        let reordered = structural_layout_state(
            "pages/layout.md",
            vec![
                (
                    80_014,
                    None,
                    "a",
                    format!("omega\nid:: {anchored}"),
                    Some(anchored),
                ),
                (80_011, None, "b", "alpha".into(), None),
                (80_013, Some(80_011), "a", "child two".into(), None),
                (80_012, Some(80_011), "b", "child one".into(), None),
            ],
        );
        let reordered_projection = reproject_with_source_identities(&base, &source, &reordered);
        assert_eq!(
            std::str::from_utf8(reordered_projection.target()).unwrap(),
            format!("- omega\n  id:: {anchored}\n- alpha\n\n\t- child two\n\t- child one\n")
        );
        let unproved = plan_projection(
            WorkspaceId::from_uuid(Uuid::from_u128(80_002)),
            &reordered,
            Some(source.as_bytes()),
        )
        .unwrap();
        assert_eq!(
            std::str::from_utf8(unproved.target()).unwrap(),
            format!("- omega\n  id:: {anchored}\n- alpha\n\t- child two\n\t- child one\n"),
            "without receipt identity, sparse source trivia must canonicalize instead of moving by ordinal"
        );

        let inserted_deleted = structural_layout_state(
            "pages/layout.md",
            vec![
                (80_011, None, "a", "alpha".into(), None),
                (80_015, Some(80_011), "a", "inserted".into(), None),
                (80_013, Some(80_011), "b", "child two".into(), None),
            ],
        );
        let inserted_projection =
            reproject_with_source_identities(&base, &source, &inserted_deleted);
        assert_eq!(
            std::str::from_utf8(inserted_projection.target()).unwrap(),
            "- alpha\n\t- inserted\n\n\t- child two\n"
        );

        let changed_in_place = structural_layout_state(
            "pages/layout.md",
            vec![
                (80_011, None, "a", "alpha".into(), None),
                (80_012, Some(80_011), "a", "child one".into(), None),
                (80_013, Some(80_011), "b", "child two edited".into(), None),
                (
                    80_014,
                    None,
                    "b",
                    format!("omega\nid:: {anchored}"),
                    Some(anchored),
                ),
            ],
        );
        let changed_projection =
            reproject_with_source_identities(&base, &source, &changed_in_place);
        assert!(std::str::from_utf8(changed_projection.target())
            .unwrap()
            .contains("child one\n\n\t- child two edited\n\n- omega"));

        for state_and_projection in [
            (&reordered, &reordered_projection),
            (&inserted_deleted, &inserted_projection),
            (&changed_in_place, &changed_projection),
        ] {
            let expected = build_projection_document(
                state_and_projection.0,
                ProjectionFormat::Markdown,
                ProjectionRenderMode::Sparse,
                None,
            )
            .unwrap();
            assert_eq!(
                crate::doc::parse(std::str::from_utf8(state_and_projection.1.target()).unwrap()),
                expected
            );
        }
    }

    #[test]
    fn org_structural_trivia_is_identity_safe_under_same_count_reorder() {
        let base = structural_layout_state(
            "pages/layout.org",
            vec![
                (80_021, None, "a", "first".into(), None),
                (80_022, Some(80_021), "a", "nested".into(), None),
                (80_023, None, "b", "last".into(), None),
            ],
        );
        let source = "* first\n\n** nested\n\n* last\n";
        let reordered = structural_layout_state(
            "pages/layout.org",
            vec![
                (80_023, None, "a", "last".into(), None),
                (80_021, None, "b", "first".into(), None),
                (80_022, Some(80_021), "a", "nested".into(), None),
            ],
        );
        let projection = reproject_with_source_identities(&base, source, &reordered);
        assert_eq!(
            std::str::from_utf8(projection.target()).unwrap(),
            "* last\n* first\n\n** nested\n"
        );
        let expected = build_projection_document(
            &reordered,
            ProjectionFormat::Org,
            ProjectionRenderMode::Sparse,
            None,
        )
        .unwrap();
        assert_eq!(
            crate::org::parse_org(std::str::from_utf8(projection.target()).unwrap()),
            expected
        );
    }

    #[test]
    fn collapsed_heading_projection_retains_parser_owned_sibling_topology() {
        let base = structural_layout_state(
            "pages/collapsed.md",
            vec![
                (80_031, None, "a", "# Parent\ncollapsed:: true".into(), None),
                (80_032, None, "b", "child".into(), None),
            ],
        );
        let source = "# Parent\ncollapsed:: true\n- child\n";
        let inserted_root = structural_layout_state(
            "pages/collapsed.md",
            vec![
                (80_033, None, "a", "new root".into(), None),
                (80_031, None, "b", "# Parent\ncollapsed:: true".into(), None),
                (80_032, None, "c", "child".into(), None),
            ],
        );
        let projection = reproject_with_source_identities(&base, source, &inserted_root);
        let projected = std::str::from_utf8(projection.target()).unwrap();
        assert!(projected.starts_with("- new root\n- # Parent\n"));
        let reparsed = crate::doc::parse(projected);
        assert_eq!(reparsed.roots.len(), 3);
        assert_eq!(reparsed.roots[0].raw, "new root");
        assert_eq!(reparsed.roots[1].raw, "# Parent\ncollapsed:: true");
        assert_eq!(reparsed.roots[2].raw, "child");

        let changed_child = structural_layout_state(
            "pages/collapsed.md",
            vec![
                (80_031, None, "a", "# Parent\ncollapsed:: true".into(), None),
                (80_032, None, "b", "child edited".into(), None),
            ],
        );
        let retained = reproject_with_source_identities(&base, source, &changed_child);
        assert!(std::str::from_utf8(retained.target())
            .unwrap()
            .starts_with("# Parent\ncollapsed:: true\n- child edited\n"));
    }

    #[test]
    fn dense_bytes_and_sparse_projection_differ_only_by_fixture_generated_anchor() {
        let markdown = {
            let home_document_id = DocumentId::from_uuid(Uuid::from_u128(1_000));
            let parent = BlockId::from_uuid(Uuid::from_u128(101));
            let duplicate_first = BlockId::from_uuid(Uuid::from_u128(102));
            let generated_sparse = BlockId::from_uuid(Uuid::from_u128(103));
            let grandchild = BlockId::from_uuid(Uuid::from_u128(104));
            let duplicate_second = BlockId::from_uuid(Uuid::from_u128(105));
            let user_authored = BlockId::from_uuid(Uuid::from_u128(106));
            let sparse_policy_id = LogseqUuid::from_uuid(Uuid::from_u128(151));
            let user_authored_id = LogseqUuid::from_uuid(Uuid::from_u128(152));
            let block_ref = LogseqUuid::from_uuid(Uuid::from_u128(153));

            DenseSparseCorpusCase {
                name: "markdown nested CRLF corpus",
                state: ProjectionPageState {
                    page: MaterializedPage {
                        page_id: PageId::from_uuid(Uuid::from_u128(1_001)),
                        home_document_id,
                        name: crate::oplog::LogicalPageName::parse("Dense Corpus").unwrap(),
                        path: ManagedPath::parse("pages/研究/Δ corpus.md").unwrap(),
                        kind: crate::oplog::ManagedTextKind::Page,
                        preamble: Some("title:: Dense Corpus\nalias:: policy corpus".into()),
                        blocks: vec![
                            MaterializedBlock {
                                block_id: user_authored,
                                home_document_id,
                                parent: None,
                                order: "z".into(),
                                logseq_uuid: Some(user_authored_id),
                                logseq_identity_origin: Some(LogseqIdentityOrigin::ExternalImported),
                                content: format!(
                                    "DONE User-owned #review [[Other Page]]\nowner:: María\nstatus:: approved\nid:: {user_authored_id}"
                                ),
                            },
                            MaterializedBlock {
                                block_id: generated_sparse,
                                home_document_id,
                                parent: Some(parent),
                                order: "b".into(),
                                logseq_uuid: Some(sparse_policy_id),
                                logseq_identity_origin: Some(LogseqIdentityOrigin::PolicyGenerated {
                                    reason: crate::oplog::PolicyGeneratedAnchorReason::BlockReference,
                                }),
                                content: "DOING generated sparse #keep [[Policy Page]]\npolicy:: retained".into(),
                            },
                            MaterializedBlock {
                                block_id: parent,
                                home_document_id,
                                parent: None,
                                order: "a".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: format!(
                                    "TODO Parent 東京 #work [[Project Alpha]] (({block_ref}))\ncontinued 🧪 line\ncustom:: keep md\nview:: table"
                                ),
                            },
                            MaterializedBlock {
                                block_id: grandchild,
                                home_document_id,
                                parent: Some(duplicate_first),
                                order: "a".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: "Grandchild line one\nline two [[Nested Page]]".into(),
                            },
                            MaterializedBlock {
                                block_id: duplicate_second,
                                home_document_id,
                                parent: Some(parent),
                                order: "c".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: "Duplicate sibling 🧩".into(),
                            },
                            MaterializedBlock {
                                block_id: duplicate_first,
                                home_document_id,
                                parent: Some(parent),
                                order: "a".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: "Duplicate sibling 🧩".into(),
                            },
                        ],
                        stats: MaterializationStats::default(),
                    },
                    frontier: FrontierV2::default(),
                    claim_evidence: Vec::new(),
                },
                base: Some(b"previous projection\r\n"),
                expected: CanonicalDocument {
                    preamble: Some("title:: Dense Corpus\nalias:: policy corpus".into()),
                    roots: vec![
                        expected_block(
                            "TODO Parent 東京 #work [[Project Alpha]] ((00000000-0000-0000-0000-000000000099))\ncontinued 🧪 line",
                            &[("custom", "keep md"), ("view", "table")],
                            Some("TODO"),
                            &["work"],
                            &["Project Alpha", "work"],
                            &["00000000-0000-0000-0000-000000000099"],
                            None,
                            None,
                            vec![
                                expected_block(
                                    "Duplicate sibling 🧩",
                                    &[],
                                    None,
                                    &[],
                                    &[],
                                    &[],
                                    None,
                                    None,
                                    vec![expected_block(
                                        "Grandchild line one\nline two [[Nested Page]]",
                                        &[],
                                        None,
                                        &[],
                                        &["Nested Page"],
                                        &[],
                                        None,
                                        None,
                                        Vec::new(),
                                    )],
                                ),
                                expected_block(
                                    "DOING generated sparse #keep [[Policy Page]]",
                                    &[
                                        ("policy", "retained"),
                                        ("id", "00000000-0000-0000-0000-000000000097"),
                                    ],
                                    Some("DOING"),
                                    &["keep"],
                                    &["Policy Page", "keep"],
                                    &[],
                                    None,
                                    None,
                                    Vec::new(),
                                ),
                                expected_block(
                                    "Duplicate sibling 🧩",
                                    &[],
                                    None,
                                    &[],
                                    &[],
                                    &[],
                                    None,
                                    None,
                                    Vec::new(),
                                ),
                            ],
                        ),
                        expected_block(
                            "DONE User-owned #review [[Other Page]]",
                            &[
                                ("owner", "María"),
                                ("status", "approved"),
                                ("id", "00000000-0000-0000-0000-000000000098"),
                            ],
                            Some("DONE"),
                            &["review"],
                            &["Other Page", "review"],
                            &[],
                            None,
                            None,
                            Vec::new(),
                        ),
                    ],
                },
                dense_only_anchors: vec![
                    DenseOnlyAnchor {
                        locator: vec![0],
                        logseq_uuid: LogseqUuid::from_uuid(parent.as_uuid()),
                    },
                    DenseOnlyAnchor {
                        locator: vec![0, 0],
                        logseq_uuid: LogseqUuid::from_uuid(duplicate_first.as_uuid()),
                    },
                    DenseOnlyAnchor {
                        locator: vec![0, 0, 0],
                        logseq_uuid: LogseqUuid::from_uuid(grandchild.as_uuid()),
                    },
                    DenseOnlyAnchor {
                        locator: vec![0, 2],
                        logseq_uuid: LogseqUuid::from_uuid(duplicate_second.as_uuid()),
                    },
                ],
                sparse_generated_anchors: vec![(generated_sparse, sparse_policy_id)],
                ordinary_idless_locators: vec![vec![0], vec![0, 0], vec![0, 0, 0], vec![0, 2]],
                user_authored_byte_fragments: vec![
                    "owner:: María".into(),
                    "status:: approved".into(),
                    format!("id:: {user_authored_id}"),
                ],
                expect_crlf: true,
            }
        };

        let org = {
            let home_document_id = DocumentId::from_uuid(Uuid::from_u128(2_000));
            let parent = BlockId::from_uuid(Uuid::from_u128(201));
            let duplicate_first = BlockId::from_uuid(Uuid::from_u128(202));
            let grandchild = BlockId::from_uuid(Uuid::from_u128(203));
            let duplicate_second = BlockId::from_uuid(Uuid::from_u128(204));
            let user_authored = BlockId::from_uuid(Uuid::from_u128(205));
            let user_authored_id = LogseqUuid::from_uuid(Uuid::from_u128(251));
            let block_ref = LogseqUuid::from_uuid(Uuid::from_u128(252));

            DenseSparseCorpusCase {
                name: "org nested LF corpus",
                state: ProjectionPageState {
                    page: MaterializedPage {
                        page_id: PageId::from_uuid(Uuid::from_u128(2_001)),
                        home_document_id,
                        name: crate::oplog::LogicalPageName::parse("Org Corpus").unwrap(),
                        path: ManagedPath::parse("journals/研究/Δ corpus.org").unwrap(),
                        kind: crate::oplog::ManagedTextKind::Page,
                        preamble: Some("#+TITLE: Org corpus\n#+PROPERTY: CATEGORY research".into()),
                        blocks: vec![
                            MaterializedBlock {
                                block_id: duplicate_second,
                                home_document_id,
                                parent: Some(parent),
                                order: "c".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: "Duplicate Org sibling 🧩".into(),
                            },
                            MaterializedBlock {
                                block_id: user_authored,
                                home_document_id,
                                parent: None,
                                order: "z".into(),
                                logseq_uuid: Some(user_authored_id),
                                logseq_identity_origin: Some(LogseqIdentityOrigin::ExternalImported),
                                content: format!(
                                    "DONE User org :review:\n:PROPERTIES:\n:owner: Κατερίνα\n:state: retained\n:id: {user_authored_id}\n:END:\nbody [[Other Org]]"
                                ),
                            },
                            MaterializedBlock {
                                block_id: parent,
                                home_document_id,
                                parent: None,
                                order: "a".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: format!(
                                    "TODO Org parent :orgtag:\nSCHEDULED: <2026-08-01 Sat>\n:PROPERTIES:\n:custom: αβ\n:END:\nmultiline 日本語 [[Linked Page][alias]] (({block_ref}))"
                                ),
                            },
                            MaterializedBlock {
                                block_id: grandchild,
                                home_document_id,
                                parent: Some(duplicate_first),
                                order: "a".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: "Org grandchild\ncontinued [[Nested Org]]".into(),
                            },
                            MaterializedBlock {
                                block_id: duplicate_first,
                                home_document_id,
                                parent: Some(parent),
                                order: "a".into(),
                                logseq_uuid: None,
                                logseq_identity_origin: None,
                                content: "Duplicate Org sibling 🧩".into(),
                            },
                        ],
                        stats: MaterializationStats::default(),
                    },
                    frontier: FrontierV2::default(),
                    claim_evidence: Vec::new(),
                },
                base: None,
                expected: CanonicalDocument {
                    preamble: Some("#+TITLE: Org corpus\n#+PROPERTY: CATEGORY research".into()),
                    roots: vec![
                        expected_block(
                            "TODO Org parent :orgtag:\nSCHEDULED: <2026-08-01 Sat>\nmultiline 日本語 [[Linked Page][alias]] ((00000000-0000-0000-0000-0000000000fc))",
                            &[("custom", "αβ")],
                            Some("TODO"),
                            &["orgtag"],
                            &["Linked Page"],
                            &["00000000-0000-0000-0000-0000000000fc"],
                            Some("2026-08-01 Sat"),
                            None,
                            vec![
                                expected_block(
                                    "Duplicate Org sibling 🧩",
                                    &[],
                                    None,
                                    &[],
                                    &[],
                                    &[],
                                    None,
                                    None,
                                    vec![expected_block(
                                        "Org grandchild\ncontinued [[Nested Org]]",
                                        &[],
                                        None,
                                        &[],
                                        &["Nested Org"],
                                        &[],
                                        None,
                                        None,
                                        Vec::new(),
                                    )],
                                ),
                                expected_block(
                                    "Duplicate Org sibling 🧩",
                                    &[],
                                    None,
                                    &[],
                                    &[],
                                    &[],
                                    None,
                                    None,
                                    Vec::new(),
                                ),
                            ],
                        ),
                        expected_block(
                            "DONE User org :review:\nbody [[Other Org]]",
                            &[
                                ("owner", "Κατερίνα"),
                                ("state", "retained"),
                                ("id", "00000000-0000-0000-0000-0000000000fb"),
                            ],
                            Some("DONE"),
                            &["review"],
                            &["Other Org"],
                            &[],
                            None,
                            None,
                            Vec::new(),
                        ),
                    ],
                },
                dense_only_anchors: vec![
                    DenseOnlyAnchor {
                        locator: vec![0],
                        logseq_uuid: LogseqUuid::from_uuid(parent.as_uuid()),
                    },
                    DenseOnlyAnchor {
                        locator: vec![0, 0],
                        logseq_uuid: LogseqUuid::from_uuid(duplicate_first.as_uuid()),
                    },
                    DenseOnlyAnchor {
                        locator: vec![0, 0, 0],
                        logseq_uuid: LogseqUuid::from_uuid(grandchild.as_uuid()),
                    },
                    DenseOnlyAnchor {
                        locator: vec![0, 1],
                        logseq_uuid: LogseqUuid::from_uuid(duplicate_second.as_uuid()),
                    },
                ],
                sparse_generated_anchors: Vec::new(),
                ordinary_idless_locators: vec![vec![0], vec![0, 0], vec![0, 0, 0], vec![0, 1]],
                user_authored_byte_fragments: vec![
                    ":owner: Κατερίνα".into(),
                    ":state: retained".into(),
                    format!(":id: {user_authored_id}"),
                ],
                expect_crlf: false,
            }
        };

        for case in [markdown, org] {
            let format = format_for_page(&case.state.page).unwrap();
            let sparse = render_projection(&case.state, case.base, None).unwrap();
            let dense = render_dense_projection_bytes(&case.state, case.base).unwrap();
            let sparse_text = std::str::from_utf8(&sparse.target).unwrap();
            let dense_text = std::str::from_utf8(&dense).unwrap();
            let sparse_semantics = canonical_semantics(format, &sparse.target, &[]);
            let dense_unfiltered = canonical_semantics(format, &dense, &[]);
            let dense_semantics = canonical_semantics(format, &dense, &case.dense_only_anchors);
            let generated_sparse: Vec<_> = sparse
                .generated_anchors
                .iter()
                .map(|anchor| (anchor.block_id(), anchor.logseq_uuid()))
                .collect();

            assert_eq!(
                generated_sparse, case.sparse_generated_anchors,
                "{} reports exactly its sparse policy-generated anchors",
                case.name
            );
            assert_eq!(
                sparse_semantics, case.expected,
                "{} sparse output must meet the explicit semantic fixture",
                case.name
            );
            assert_eq!(
                dense_semantics, case.expected,
                "{} dense output must meet the explicit semantic fixture after removing only listed dense anchors",
                case.name
            );
            assert_eq!(
                sparse_semantics, dense_semantics,
                "{} dense and sparse semantics must agree after the narrow normalization",
                case.name
            );
            for anchor in &case.dense_only_anchors {
                assert!(
                    has_id(
                        block_at(&dense_unfiltered.roots, &anchor.locator),
                        anchor.logseq_uuid
                    ),
                    "{} dense output must contain its listed fixture-only anchor at {:?}",
                    case.name,
                    anchor.locator
                );
                assert!(
                    !has_id(
                        block_at(&sparse_semantics.roots, &anchor.locator),
                        anchor.logseq_uuid
                    ),
                    "{} sparse output must not inherit its fixture-only anchor at {:?}",
                    case.name,
                    anchor.locator
                );
            }
            for locator in &case.ordinary_idless_locators {
                assert!(
                    !block_at(&sparse_semantics.roots, locator)
                        .properties
                        .iter()
                        .any(|(key, _)| key.eq_ignore_ascii_case("id")),
                    "{} sparse output must leave the ordinary ID-less block at {locator:?} unstamped",
                    case.name
                );
            }
            for fragment in &case.user_authored_byte_fragments {
                assert!(
                    sparse_text.contains(fragment) && dense_text.contains(fragment),
                    "{} must preserve user-authored bytes {fragment:?}",
                    case.name
                );
            }
            assert_expected_line_endings(case.name, &sparse.target, case.expect_crlf);
            assert_expected_line_endings(case.name, &dense, case.expect_crlf);
        }
    }

    #[test]
    fn projection_format_accepts_mixed_case_markdown_and_org_without_output_changes() {
        fn page(path: &str) -> MaterializedPage {
            MaterializedPage {
                page_id: PageId::from_uuid(Uuid::from_u128(11)),
                home_document_id: DocumentId::from_uuid(Uuid::from_u128(12)),
                name: crate::oplog::LogicalPageName::parse("Format").unwrap(),
                path: ManagedPath::parse(path).unwrap(),
                kind: crate::oplog::ManagedTextKind::Page,
                preamble: Some("title:: Format".into()),
                blocks: vec![MaterializedBlock {
                    block_id: BlockId::from_uuid(Uuid::from_u128(13)),
                    home_document_id: DocumentId::from_uuid(Uuid::from_u128(12)),
                    parent: None,
                    order: "a".into(),
                    logseq_uuid: None,
                    logseq_identity_origin: None,
                    content: "content".into(),
                }],
                stats: MaterializationStats::default(),
            }
        }

        let markdown = ProjectionPageState {
            page: page("Root.md"),
            frontier: FrontierV2::default(),
            claim_evidence: Vec::new(),
        };
        let mixed_markdown = ProjectionPageState {
            page: page("Root.MaRkDoWn"),
            frontier: FrontierV2::default(),
            claim_evidence: Vec::new(),
        };
        assert_eq!(
            render_projection(&markdown, None, None).unwrap().target,
            render_projection(&mixed_markdown, None, None)
                .unwrap()
                .target
        );

        let org = ProjectionPageState {
            page: page("Root.org"),
            frontier: FrontierV2::default(),
            claim_evidence: Vec::new(),
        };
        let mixed_org = ProjectionPageState {
            page: page("Root.OrG"),
            frontier: FrontierV2::default(),
            claim_evidence: Vec::new(),
        };
        assert_eq!(
            render_projection(&org, None, None).unwrap().target,
            render_projection(&mixed_org, None, None).unwrap().target
        );
    }

    #[test]
    fn manifested_projection_fault_scope_cleans_up_after_unwind_and_consumption() {
        assert!(fail_during_manifested_projection_for_harness().is_ok());

        let unwind = std::panic::catch_unwind(|| {
            let _fault_scope = fail_next_manifested_projection_during_write_for_harness();
            panic!("deterministic unwind before manifested projection");
        });
        assert!(unwind.is_err());
        assert!(fail_during_manifested_projection_for_harness().is_ok());

        let fault_scope = fail_next_manifested_projection_during_write_for_harness();
        assert!(fail_during_manifested_projection_for_harness().is_err());
        drop(fault_scope);
        assert!(fail_during_manifested_projection_for_harness().is_ok());
    }
}
