//! Complete, disposable graph-wide materialization behind the SQLite frontier.
//!
//! The types in this module are an adapter boundary, not a second authority.
//! An accepted semantic effect does not currently contain parser-derived names,
//! references, properties, tags, task facets, formatting facets, or searchable
//! text.  Callers must therefore provide those values explicitly from an
//! authoritative post-acceptance snapshot.  The SQLite applier validates the
//! input against the accepted semantic effect and applies it in the same SQL
//! transaction that advances the accepted frontier.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[cfg(test)]
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use tine_storage::sqlite as storage;
use uuid::Uuid;

use super::{
    AcceptedBatchEvent, BatchId, BlockId, BlockOwner, ContentDigest, DocumentId,
    LogseqIdentityOrigin, LogseqUuid, ManagedPath, ManagedTextKind, PageId, PageState,
    PolicyGeneratedAnchorReason, ReferenceCatalogRootV2, ReferenceSourceLocatorV1, SemanticEffect,
    REFERENCE_CATALOG_EXTRACTOR_VERSION, REFERENCE_CATALOG_POLICY_VERSION,
};

pub const MAX_MATERIALIZATION_QUERY_ROWS: usize = storage::MAX_MATERIALIZATION_QUERY_ROWS;
/// Largest accepted materialization string other than a page preamble.
///
/// This retains the established semantic block-content capacity while keeping
/// individual SQLite/FTS values bounded.
pub const MAX_MATERIALIZATION_FIELD_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MATERIALIZATION_PREAMBLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MATERIALIZATION_FACET_VALUES: usize = 16_384;
pub const MAX_MATERIALIZATION_FACET_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MATERIALIZATION_CHANGE_PAGES: usize = 65_536;
pub const MAX_MATERIALIZATION_CHANGE_BLOCKS: usize = 262_144;
pub const MAX_MATERIALIZATION_CHANGE_FACET_VALUES: usize = 1_048_576;
pub const MAX_MATERIALIZATION_CHANGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_MATERIALIZATION_QUERY_BYTES: usize = storage::MAX_MATERIALIZATION_QUERY_BYTES;
pub const MAX_MATERIALIZATION_READ_BYTES: usize = storage::MAX_MATERIALIZATION_READ_BYTES;

const MATERIALIZATION_PAGE_OVERHEAD_BYTES: usize = 96;
const MATERIALIZATION_BLOCK_OVERHEAD_BYTES: usize = 128;
const MATERIALIZATION_REFERENCE_OVERHEAD_BYTES: usize = 48;
const MATERIALIZATION_PROPERTY_OVERHEAD_BYTES: usize = 24;
const MATERIALIZATION_TAG_OVERHEAD_BYTES: usize = 16;
const MATERIALIZATION_STRING_OVERHEAD_BYTES: usize = 16;
const REFERENCE_CATALOG_POSTING_OVERHEAD_BYTES: usize = 96;
const REFERENCE_CATALOG_ALIAS_OVERHEAD_BYTES: usize = 80;
const REFERENCE_CATALOG_BINDING_OVERHEAD_BYTES: usize = 64;
const REFERENCE_CATALOG_COVERAGE_OVERHEAD_BYTES: usize = 80;
// Packet 3 attaches the already-authenticated reference-catalog transition to
// the same SQL transaction as the ordinary page materialization. The
// authenticated reverse-candidate contract is persisted as SQLite schema v10;
// schema v11 adds page-led cleanup and authoritative FTS ownership.
const MATERIALIZATION_INPUT_SCHEMA_VERSION: u32 = 4;
pub(crate) const REFERENCE_EXTRACTOR_DEPENDENCY_STAMP_SCHEMA_VERSION: u32 = 2;
const REFERENCE_EXTRACTOR_DEPENDENCY_STAMP_DOMAIN: &[u8] =
    b"tine/sqlite-reference-extractor-dependency-stamp/v2";

pub(crate) type ApplyChangeInstrumentation = storage::ApplyChangeInstrumentation;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializedEntityId {
    Page(PageId),
    Block(BlockId),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializedReferenceKind {
    Reference,
    Embed,
    TagReference,
    PropertyReference,
}

impl MaterializedReferenceKind {
    const fn sql_value(self) -> i64 {
        match self {
            Self::Reference => 0,
            Self::Embed => 1,
            Self::TagReference => 2,
            Self::PropertyReference => 3,
        }
    }

    fn from_sql(value: i64) -> Result<Self, MaterializationError> {
        match value {
            0 => Ok(Self::Reference),
            1 => Ok(Self::Embed),
            2 => Ok(Self::TagReference),
            3 => Ok(Self::PropertyReference),
            _ => Err(MaterializationError::Corrupt(format!(
                "unknown reference kind {value}"
            ))),
        }
    }
}

/// SQLite's crate-private representation of Packet 1 reference kinds.
///
/// It deliberately does not make Packet 1 depend on SQLite. The authenticated
/// catalog adapter in the following slice is the only boundary that may turn
/// Packet 1 facts into these values.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReferenceCatalogReferenceKind {
    PageLink,
    Tag,
    PageEmbed,
    LinkablePropertyValue,
    AliasDeclaration,
    PropertyKeyPseudoPage,
    BlockReference,
    BlockEmbed,
}

impl ReferenceCatalogReferenceKind {
    pub(crate) const fn from_page_kind(kind: super::PageReferenceKindV1) -> Self {
        match kind {
            super::PageReferenceKindV1::PageLink => Self::PageLink,
            super::PageReferenceKindV1::Tag => Self::Tag,
            super::PageReferenceKindV1::PageEmbed => Self::PageEmbed,
            super::PageReferenceKindV1::LinkablePropertyValue => Self::LinkablePropertyValue,
            super::PageReferenceKindV1::AliasDeclaration => Self::AliasDeclaration,
            super::PageReferenceKindV1::PropertyKeyPseudoPage => Self::PropertyKeyPseudoPage,
        }
    }

    pub(crate) const fn from_block_kind(kind: super::BlockReferenceKindV1) -> Self {
        match kind {
            super::BlockReferenceKindV1::Reference => Self::BlockReference,
            super::BlockReferenceKindV1::Embed => Self::BlockEmbed,
        }
    }

    pub(crate) const fn sql_value(self) -> i64 {
        match self {
            Self::PageLink => 0,
            Self::Tag => 1,
            Self::PageEmbed => 2,
            Self::LinkablePropertyValue => 3,
            Self::AliasDeclaration => 4,
            Self::PropertyKeyPseudoPage => 5,
            Self::BlockReference => 6,
            Self::BlockEmbed => 7,
        }
    }

    const fn accepts_target(self, target: &MaterializedReferenceTarget) -> bool {
        matches!(
            (self, target),
            (
                Self::PageLink
                    | Self::Tag
                    | Self::PageEmbed
                    | Self::LinkablePropertyValue
                    | Self::AliasDeclaration
                    | Self::PropertyKeyPseudoPage,
                MaterializedReferenceTarget::PageName { .. }
            ) | (
                Self::BlockReference | Self::BlockEmbed,
                MaterializedReferenceTarget::ExternalUuid { .. }
            )
        )
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum MaterializedReferenceTarget {
    PageName {
        raw_name: String,
        normalized_name: String,
        resolved_page_id: Option<PageId>,
    },
    ExternalUuid {
        raw_claim: LogseqUuid,
        resolved_block_id: Option<BlockId>,
    },
}

impl MaterializedReferenceTarget {
    fn validate(
        &self,
        input_budget: &mut MaterializationInputBudget,
    ) -> Result<(), MaterializationError> {
        match self {
            Self::PageName {
                raw_name,
                normalized_name,
                ..
            } => {
                validate_page_name_pair("reference target", raw_name, normalized_name)?;
                input_budget.add_field(
                    "reference raw name bytes",
                    raw_name,
                    MAX_MATERIALIZATION_FIELD_BYTES,
                )?;
                input_budget.add_field(
                    "reference normalized name bytes",
                    normalized_name,
                    MAX_MATERIALIZATION_FIELD_BYTES,
                )?;
            }
            Self::ExternalUuid { .. } => input_budget.add_bytes(16)?,
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferenceExtractorDependencyStamp {
    schema_version: u32,
    extractor_version: u32,
    extractor_digest: ContentDigest,
    policy_version: u32,
    policy_digest: ContentDigest,
}

impl ReferenceExtractorDependencyStamp {
    pub(crate) fn new(
        extractor_digest: ContentDigest,
        policy_digest: ContentDigest,
    ) -> Result<Self, MaterializationError> {
        let stamp = Self {
            schema_version: REFERENCE_EXTRACTOR_DEPENDENCY_STAMP_SCHEMA_VERSION,
            extractor_version: REFERENCE_CATALOG_EXTRACTOR_VERSION,
            extractor_digest,
            policy_version: REFERENCE_CATALOG_POLICY_VERSION,
            policy_digest,
        };
        stamp.validate()?;
        Ok(stamp)
    }

    fn validate(&self) -> Result<(), MaterializationError> {
        if self.schema_version != REFERENCE_EXTRACTOR_DEPENDENCY_STAMP_SCHEMA_VERSION
            || self.extractor_version != REFERENCE_CATALOG_EXTRACTOR_VERSION
            || self.policy_version != REFERENCE_CATALOG_POLICY_VERSION
        {
            return Err(MaterializationError::InvalidInput(
                "unknown reference extractor dependency stamp version".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn digest(&self) -> Result<ContentDigest, MaterializationError> {
        self.validate()?;
        let encoded = postcard::to_allocvec(self)
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        let mut preimage = Vec::with_capacity(
            REFERENCE_EXTRACTOR_DEPENDENCY_STAMP_DOMAIN.len() + 1 + encoded.len(),
        );
        preimage.extend_from_slice(REFERENCE_EXTRACTOR_DEPENDENCY_STAMP_DOMAIN);
        preimage.push(0);
        preimage.extend_from_slice(&encoded);
        Ok(ContentDigest::of(&preimage))
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterializedReferencePosting {
    pub(crate) source_page_id: PageId,
    pub(crate) source_entity: MaterializedEntityId,
    pub(crate) source_locator: ReferenceSourceLocatorV1,
    pub(crate) ordinal: u32,
    pub(crate) kind: ReferenceCatalogReferenceKind,
    pub(crate) target: MaterializedReferenceTarget,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterializedAliasDeclaration {
    pub(crate) source_page_id: PageId,
    pub(crate) source_entity: MaterializedEntityId,
    pub(crate) source_locator: ReferenceSourceLocatorV1,
    pub(crate) ordinal: u32,
    pub(crate) raw_alias: String,
    pub(crate) normalized_alias: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceCoverageFacet {
    pub(crate) source_page_id: PageId,
    pub(crate) source_digest: ContentDigest,
    pub(crate) extractor_dependency_stamp: ReferenceExtractorDependencyStamp,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterializedReferenceNameBinding {
    pub(crate) raw_name: String,
    pub(crate) normalized_name: String,
    pub(crate) candidate_ordinal: u32,
    pub(crate) resolved_page_id: Option<PageId>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterializedReferenceUuidBinding {
    pub(crate) raw_uuid_claim: LogseqUuid,
    pub(crate) candidate_ordinal: u32,
    pub(crate) resolved_block_id: Option<BlockId>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterializedReferenceAliasBinding {
    pub(crate) normalized_alias: String,
    pub(crate) candidate_ordinal: u32,
    pub(crate) resolved_page_id: Option<PageId>,
    pub(crate) catalog_root_digest: ContentDigest,
}

/// The accepted evidence which binds a catalog transition to a particular
/// frontier step.  It is deliberately separate from SQLite-shaped input: a
/// rebuilt projection must recover this only from authenticated history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedReferenceMaterialization {
    pub(crate) event_binding_digest: ContentDigest,
    pub(crate) prior_frontier_root_digest: ContentDigest,
    pub(crate) post_frontier_root_digest: ContentDigest,
    pub(crate) prior_catalog_root: ReferenceCatalogRootV2,
    pub(crate) post_catalog_root: ReferenceCatalogRootV2,
}

/// Fully validated catalog rows that may be projected only with matching
/// accepted frontier evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferenceCatalogMaterializationInput {
    prior_catalog_root: ReferenceCatalogRootV2,
    post_catalog_root: ReferenceCatalogRootV2,
    postings: Vec<MaterializedReferencePosting>,
    aliases: Vec<MaterializedAliasDeclaration>,
    name_bindings: Vec<MaterializedReferenceNameBinding>,
    uuid_bindings: Vec<MaterializedReferenceUuidBinding>,
    alias_bindings: Vec<MaterializedReferenceAliasBinding>,
    coverage: Vec<SourceCoverageFacet>,
    removed_sources: Vec<PageId>,
}

impl ReferenceCatalogMaterializationInput {
    pub(crate) fn new(
        prior_catalog_root: ReferenceCatalogRootV2,
        post_catalog_root: ReferenceCatalogRootV2,
        mut postings: Vec<MaterializedReferencePosting>,
        mut aliases: Vec<MaterializedAliasDeclaration>,
        mut name_bindings: Vec<MaterializedReferenceNameBinding>,
        mut uuid_bindings: Vec<MaterializedReferenceUuidBinding>,
        mut alias_bindings: Vec<MaterializedReferenceAliasBinding>,
        mut coverage: Vec<SourceCoverageFacet>,
        mut removed_sources: Vec<PageId>,
    ) -> Result<Self, MaterializationError> {
        postings.sort_unstable();
        aliases.sort_unstable();
        name_bindings.sort_unstable();
        uuid_bindings.sort_unstable();
        alias_bindings.sort_unstable();
        coverage.sort_unstable();
        removed_sources.sort_unstable();
        let input = Self {
            prior_catalog_root,
            post_catalog_root,
            postings,
            aliases,
            name_bindings,
            uuid_bindings,
            alias_bindings,
            coverage,
            removed_sources,
        };
        input.validate()?;
        Ok(input)
    }

    fn validate(&self) -> Result<(), MaterializationError> {
        self.prior_catalog_root
            .encode()
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        self.post_catalog_root
            .encode()
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        if !strictly_sorted_unique_by(&self.postings, |posting| {
            (
                posting.source_page_id,
                posting.source_entity.clone(),
                posting.source_locator,
                posting.ordinal,
            )
        }) {
            return Err(MaterializationError::InvalidInput(
                "reference postings are not canonical".into(),
            ));
        }
        if !strictly_sorted_unique_by(&self.aliases, |alias| {
            (
                alias.source_page_id,
                alias.source_entity.clone(),
                alias.source_locator,
                alias.ordinal,
            )
        }) {
            return Err(MaterializationError::InvalidInput(
                "reference alias declarations are not canonical".into(),
            ));
        }
        if !strictly_sorted_unique_by(&self.name_bindings, |binding| {
            (binding.raw_name.clone(), binding.candidate_ordinal)
        }) {
            return Err(MaterializationError::InvalidInput(
                "reference name bindings are not canonical".into(),
            ));
        }
        if !strictly_sorted_unique_by(&self.uuid_bindings, |binding| {
            (binding.raw_uuid_claim, binding.candidate_ordinal)
        }) {
            return Err(MaterializationError::InvalidInput(
                "reference UUID bindings are not canonical".into(),
            ));
        }
        if !strictly_sorted_unique_by(&self.alias_bindings, |binding| {
            (
                binding.normalized_alias.clone(),
                binding.candidate_ordinal,
                binding.catalog_root_digest,
            )
        }) {
            return Err(MaterializationError::InvalidInput(
                "reference alias bindings are not canonical".into(),
            ));
        }
        if !strictly_sorted_unique_by(&self.coverage, |facet| facet.source_page_id) {
            return Err(MaterializationError::InvalidInput(
                "reference source coverage is not canonical".into(),
            ));
        }
        if !strictly_sorted_unique_by(&self.removed_sources, |page_id| *page_id) {
            return Err(MaterializationError::InvalidInput(
                "removed reference sources are not canonical".into(),
            ));
        }

        let covered = self
            .coverage
            .iter()
            .map(|facet| facet.source_page_id)
            .collect::<BTreeSet<_>>();
        if self
            .removed_sources
            .iter()
            .any(|page_id| covered.contains(page_id))
        {
            return Err(MaterializationError::InvalidInput(
                "one reference source is both covered and removed".into(),
            ));
        }

        let mut input_budget = MaterializationInputBudget::default();
        for posting in &self.postings {
            if !covered.contains(&posting.source_page_id) {
                return Err(MaterializationError::InvalidInput(
                    "reference posting has no source coverage".into(),
                ));
            }
            validate_reference_posting(posting, &mut input_budget)?;
        }
        for alias in &self.aliases {
            if !covered.contains(&alias.source_page_id) {
                return Err(MaterializationError::InvalidInput(
                    "reference alias has no source coverage".into(),
                ));
            }
            validate_alias_declaration(alias, &mut input_budget)?;
        }
        for binding in &self.name_bindings {
            validate_page_name_pair(
                "reference name binding",
                &binding.raw_name,
                &binding.normalized_name,
            )?;
            input_budget.add_facet_values(1)?;
            input_budget.add_bytes(REFERENCE_CATALOG_BINDING_OVERHEAD_BYTES)?;
            input_budget.add_field(
                "reference name binding raw bytes",
                &binding.raw_name,
                MAX_MATERIALIZATION_FIELD_BYTES,
            )?;
            input_budget.add_field(
                "reference name binding normalized bytes",
                &binding.normalized_name,
                MAX_MATERIALIZATION_FIELD_BYTES,
            )?;
        }
        for _binding in &self.uuid_bindings {
            input_budget.add_facet_values(1)?;
            input_budget.add_bytes(REFERENCE_CATALOG_BINDING_OVERHEAD_BYTES + 16)?;
        }
        for binding in &self.alias_bindings {
            validate_normalized_page_name("reference alias binding", &binding.normalized_alias)?;
            input_budget.add_facet_values(1)?;
            input_budget.add_bytes(REFERENCE_CATALOG_BINDING_OVERHEAD_BYTES)?;
            input_budget.add_field(
                "reference alias binding normalized bytes",
                &binding.normalized_alias,
                MAX_MATERIALIZATION_FIELD_BYTES,
            )?;
        }
        for facet in &self.coverage {
            facet.extractor_dependency_stamp.validate()?;
            let _ = facet.extractor_dependency_stamp.digest()?;
            input_budget.add_facet_values(1)?;
            input_budget.add_bytes(REFERENCE_CATALOG_COVERAGE_OVERHEAD_BYTES)?;
        }
        Ok(())
    }

    fn validate_for_authenticated_transition(
        &self,
        authenticated: &AuthenticatedReferenceMaterialization,
        effect: &SemanticEffect,
    ) -> Result<(), MaterializationError> {
        self.validate()?;
        if self.prior_catalog_root != authenticated.prior_catalog_root
            || self.post_catalog_root != authenticated.post_catalog_root
        {
            return Err(MaterializationError::Contradiction(
                "reference catalog input is not bound to the accepted frontier transition".into(),
            ));
        }
        let expected = super::reference_catalog::affected_reference_sources(effect);
        let supplied = self
            .coverage
            .iter()
            .map(|facet| facet.source_page_id)
            .chain(self.removed_sources.iter().copied())
            .collect::<BTreeSet<_>>();
        if supplied != expected {
            return Err(MaterializationError::Incomplete(
                "reference catalog input does not cover the accepted affected sources".into(),
            ));
        }
        Ok(())
    }

    fn validate_for_event(
        &self,
        event: &AcceptedBatchEvent,
        effect: &SemanticEffect,
    ) -> Result<(), MaterializationError> {
        let authenticated = AuthenticatedReferenceMaterialization {
            event_binding_digest: event.event_binding_digest(),
            prior_frontier_root_digest: ContentDigest::of(
                &postcard::to_allocvec(event.prior_frontier_root())
                    .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?,
            ),
            post_frontier_root_digest: ContentDigest::of(
                &postcard::to_allocvec(event.post_frontier_root())
                    .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?,
            ),
            prior_catalog_root: event.prior_frontier_root().reference_catalog_root().clone(),
            post_catalog_root: event.post_frontier_root().reference_catalog_root().clone(),
        };
        self.validate_for_authenticated_transition(&authenticated, effect)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedReference {
    pub target: MaterializedEntityId,
    pub kind: MaterializedReferenceKind,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedProperty {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedTask {
    pub marker: String,
    pub priority: Option<String>,
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedBlockInput {
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
    pub parent: Option<BlockId>,
    pub order: String,
    pub content: String,
    pub searchable_text: String,
    pub heading_level: Option<u8>,
    pub collapsed: bool,
    pub logseq_uuid: Option<LogseqUuid>,
    pub logseq_identity_origin: Option<LogseqIdentityOrigin>,
    pub references: Vec<MaterializedReference>,
    pub properties: Vec<MaterializedProperty>,
    pub tags: Vec<String>,
    pub task: Option<MaterializedTask>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedPageInput {
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub name: String,
    pub name_key: String,
    pub path: ManagedPath,
    pub kind: ManagedTextKind,
    pub preamble: Option<String>,
    pub searchable_text: String,
    pub references: Vec<MaterializedReference>,
    pub properties: Vec<MaterializedProperty>,
    pub tags: Vec<String>,
    pub blocks: Vec<MaterializedBlockInput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializationChange {
    schema_version: u32,
    batch_id: BatchId,
    replacements: Vec<MaterializedPageInput>,
    deletions: Vec<PageId>,
    reference_catalog: Option<ReferenceCatalogMaterializationInput>,
}

impl MaterializationChange {
    pub fn new(
        batch_id: BatchId,
        mut replacements: Vec<MaterializedPageInput>,
        mut deletions: Vec<PageId>,
    ) -> Result<Self, MaterializationError> {
        let mut input_budget = MaterializationInputBudget::default();
        input_budget.add_pages(replacements.len())?;
        input_budget.add_pages(deletions.len())?;
        replacements.sort_unstable_by_key(|page| page.page_id);
        deletions.sort_unstable();
        let change = Self {
            schema_version: MATERIALIZATION_INPUT_SCHEMA_VERSION,
            batch_id,
            replacements,
            deletions,
            reference_catalog: None,
        };
        change.validate_shape()?;
        Ok(change)
    }

    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub fn replacements(&self) -> &[MaterializedPageInput] {
        &self.replacements
    }

    pub fn deletions(&self) -> &[PageId] {
        &self.deletions
    }

    /// Attach the catalog transition that was independently authenticated by
    /// an accepted batch.  This is crate-private so callers cannot make a
    /// SQLite row claim a catalog root without the engine/store adapter.
    pub(crate) fn with_authenticated_reference_catalog(
        mut self,
        reference_catalog: ReferenceCatalogMaterializationInput,
    ) -> Result<Self, MaterializationError> {
        if self.reference_catalog.is_some() {
            return Err(MaterializationError::InvalidInput(
                "materialization change already has a reference catalog transition".into(),
            ));
        }
        self.reference_catalog = Some(reference_catalog);
        self.validate_shape()?;
        Ok(self)
    }

    pub(crate) fn reference_catalog(&self) -> Option<&ReferenceCatalogMaterializationInput> {
        self.reference_catalog.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn without_reference_catalog(mut self) -> Result<Self, MaterializationError> {
        self.reference_catalog = None;
        self.validate_shape()?;
        Ok(self)
    }

    pub fn digest(&self) -> Result<ContentDigest, MaterializationError> {
        self.validate_shape()?;
        let encoded = postcard::to_allocvec(self)
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        if encoded.len() > MAX_MATERIALIZATION_CHANGE_BYTES {
            return Err(resource_limit(
                "materialization change bytes",
                encoded.len(),
                MAX_MATERIALIZATION_CHANGE_BYTES,
            ));
        }
        Ok(ContentDigest::of(&encoded))
    }

    pub(crate) fn validate_for_event(
        &self,
        event: &AcceptedBatchEvent,
    ) -> Result<ContentDigest, MaterializationError> {
        if self.batch_id != event.batch_id() {
            return Err(MaterializationError::BatchMismatch {
                expected: event.batch_id(),
                found: self.batch_id,
            });
        }
        let effect = SemanticEffect::decode(event.semantic_effect())
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        self.validate_against_effect(&effect)?;
        if let Some(reference_catalog) = &self.reference_catalog {
            reference_catalog.validate_for_event(event, &effect)?;
        }
        self.digest()
    }

    #[cfg(test)]
    pub(crate) fn validate_against_stored(
        &self,
        batch_id: BatchId,
        semantic_effect: &[u8],
    ) -> Result<ContentDigest, MaterializationError> {
        if self.batch_id != batch_id {
            return Err(MaterializationError::BatchMismatch {
                expected: batch_id,
                found: self.batch_id,
            });
        }
        let effect = SemanticEffect::decode(semantic_effect)
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        self.validate_against_effect(&effect)?;
        if let Some(reference_catalog) = &self.reference_catalog {
            reference_catalog.validate()?;
        }
        self.digest()
    }

    fn validate_shape(&self) -> Result<(), MaterializationError> {
        if self.schema_version != MATERIALIZATION_INPUT_SCHEMA_VERSION {
            return Err(MaterializationError::InvalidInput(format!(
                "unknown materialization input schema {}",
                self.schema_version
            )));
        }
        if !strictly_sorted_unique_by(&self.replacements, |page| page.page_id)
            || !strictly_sorted_unique_by(&self.deletions, |page_id| *page_id)
        {
            return Err(MaterializationError::InvalidInput(
                "page replacements/deletions are not canonical".into(),
            ));
        }
        let mut input_budget = MaterializationInputBudget::default();
        input_budget.add_pages(self.replacements.len())?;
        input_budget.add_pages(self.deletions.len())?;
        input_budget.add_bytes(self.deletions.len().checked_mul(32).ok_or_else(|| {
            resource_limit(
                "materialization change bytes",
                usize::MAX,
                MAX_MATERIALIZATION_CHANGE_BYTES,
            )
        })?)?;
        let replacement_ids = self
            .replacements
            .iter()
            .map(|page| page.page_id)
            .collect::<BTreeSet<_>>();
        if self
            .deletions
            .iter()
            .any(|page_id| replacement_ids.contains(page_id))
        {
            return Err(MaterializationError::InvalidInput(
                "one page is both replaced and deleted".into(),
            ));
        }
        let mut block_ids = BTreeSet::new();
        for page in &self.replacements {
            validate_page(page, &mut input_budget)?;
            for block in &page.blocks {
                if !block_ids.insert(block.block_id) {
                    return Err(MaterializationError::InvalidInput(format!(
                        "block {} occurs in multiple replacement pages",
                        block.block_id
                    )));
                }
            }
        }
        if let Some(reference_catalog) = &self.reference_catalog {
            reference_catalog.validate()?;
        }
        Ok(())
    }

    fn validate_against_effect(&self, effect: &SemanticEffect) -> Result<(), MaterializationError> {
        self.validate_shape()?;
        let replacements = self
            .replacements
            .iter()
            .map(|page| (page.page_id, page))
            .collect::<BTreeMap<_, _>>();
        // `validate_shape` above rejects duplicate IDs, so this canonical
        // per-page index preserves the prior membership/block lookup semantics.
        let replacement_blocks = self
            .replacements
            .iter()
            .map(|page| {
                (
                    page.page_id,
                    page.blocks
                        .iter()
                        .map(|block| (block.block_id, block))
                        .collect::<BTreeMap<_, _>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let deletions = self.deletions.iter().copied().collect::<BTreeSet<_>>();
        let mut affected = BTreeSet::new();
        let mut required_deletions = BTreeSet::new();

        for delta in effect.pages() {
            affected.insert(delta.page_id);
            match delta.after.as_ref() {
                Some(PageState::Live {
                    name,
                    path,
                    home_document_id,
                    kind,
                }) => {
                    let page = replacements.get(&delta.page_id).ok_or_else(|| {
                        MaterializationError::Incomplete(format!(
                            "live page {} has no complete replacement",
                            delta.page_id
                        ))
                    })?;
                    let expected_name_key = name.canonical_key();
                    if page.name.as_str() != name.as_str()
                        || page.name_key.as_str() != expected_name_key.as_str()
                        || &page.path != path
                        || page.home_document_id != *home_document_id
                        || page.kind != *kind
                    {
                        return Err(MaterializationError::Contradiction(format!(
                            "page {} replacement differs from accepted name/key/path/kind/home",
                            delta.page_id
                        )));
                    }
                }
                Some(PageState::Tombstone { .. }) => {
                    required_deletions.insert(delta.page_id);
                }
                None => {
                    return Err(MaterializationError::Incomplete(format!(
                        "accepted page {} has no post-state",
                        delta.page_id
                    )));
                }
            }
        }
        for delta in effect.page_preambles() {
            affected.insert(delta.page_id);
            let page = replacements.get(&delta.page_id).ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "preamble change for page {} has no replacement",
                    delta.page_id
                ))
            })?;
            let after = delta.after.as_ref().ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "preamble change for page {} has no post-state",
                    delta.page_id
                ))
            })?;
            if page.home_document_id != after.home_document_id || page.preamble != after.preamble {
                return Err(MaterializationError::Contradiction(format!(
                    "page {} replacement differs from accepted preamble",
                    delta.page_id
                )));
            }
        }
        for delta in effect.memberships() {
            affected.insert(delta.page_id);
            if required_deletions.contains(&delta.page_id) {
                continue;
            }
            let blocks = replacement_blocks.get(&delta.page_id).ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "membership change for page {} has no replacement",
                    delta.page_id
                ))
            })?;
            match delta.after.as_ref() {
                Some(after) => {
                    let block = blocks.get(&delta.block_id).ok_or_else(|| {
                        MaterializationError::Contradiction(format!(
                            "accepted member {} is absent from page {}",
                            delta.block_id, delta.page_id
                        ))
                    })?;
                    if block.home_document_id != after.home_document_id
                        || block.parent != after.parent
                        || block.order != after.order
                    {
                        return Err(MaterializationError::Contradiction(format!(
                            "member {} differs from accepted parent/order/home",
                            delta.block_id
                        )));
                    }
                }
                None if blocks.contains_key(&delta.block_id) => {
                    return Err(MaterializationError::Contradiction(format!(
                        "removed member {} remains on page {}",
                        delta.block_id, delta.page_id
                    )));
                }
                None => {}
            }
        }
        for delta in effect.blocks() {
            let owner = delta
                .after
                .as_ref()
                .and_then(block_owner_page)
                .or_else(|| delta.before.as_ref().and_then(block_owner_page));
            let Some(page_id) = owner else {
                continue;
            };
            affected.insert(page_id);
            if required_deletions.contains(&page_id) {
                continue;
            }
            let blocks = replacement_blocks.get(&page_id).ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "block change for page {page_id} has no replacement"
                ))
            })?;
            match delta.after.as_ref() {
                Some(after) if matches!(after.owner, BlockOwner::Page(owner) if owner == page_id) =>
                {
                    let block = blocks.get(&delta.block_id).ok_or_else(|| {
                        MaterializationError::Contradiction(format!(
                            "accepted live block {} is absent from page {page_id}",
                            delta.block_id
                        ))
                    })?;
                    if block.home_document_id != after.home_document_id
                        || block.content != after.content
                        || block.logseq_uuid != after.logseq_uuid
                        || block.logseq_identity_origin != after.logseq_identity_origin
                    {
                        return Err(MaterializationError::Contradiction(format!(
                            "block {} replacement differs from accepted state",
                            delta.block_id
                        )));
                    }
                }
                Some(_) | None if blocks.contains_key(&delta.block_id) => {
                    return Err(MaterializationError::Contradiction(format!(
                        "non-live block {} remains on page {page_id}",
                        delta.block_id
                    )));
                }
                Some(_) | None => {}
            }
        }

        let supplied = replacements
            .keys()
            .copied()
            .chain(deletions.iter().copied())
            .collect::<BTreeSet<_>>();
        if supplied != affected {
            return Err(MaterializationError::Incomplete(format!(
                "supplied pages {supplied:?} differ from accepted affected pages {affected:?}"
            )));
        }
        if deletions != required_deletions {
            return Err(MaterializationError::Contradiction(format!(
                "supplied deletions {deletions:?} differ from accepted deletions {required_deletions:?}"
            )));
        }
        Ok(())
    }
}

#[derive(Default)]
struct MaterializationInputBudget {
    bytes: usize,
    pages: usize,
    blocks: usize,
    facet_values: usize,
}

impl MaterializationInputBudget {
    fn add_bytes(&mut self, bytes: usize) -> Result<(), MaterializationError> {
        self.bytes = checked_budget_add(
            "materialization change bytes",
            self.bytes,
            bytes,
            MAX_MATERIALIZATION_CHANGE_BYTES,
        )?;
        Ok(())
    }

    fn add_pages(&mut self, pages: usize) -> Result<(), MaterializationError> {
        self.pages = checked_budget_add(
            "materialization change pages",
            self.pages,
            pages,
            MAX_MATERIALIZATION_CHANGE_PAGES,
        )?;
        Ok(())
    }

    fn add_blocks(&mut self, blocks: usize) -> Result<(), MaterializationError> {
        self.blocks = checked_budget_add(
            "materialization change blocks",
            self.blocks,
            blocks,
            MAX_MATERIALIZATION_CHANGE_BLOCKS,
        )?;
        Ok(())
    }

    fn add_facet_values(&mut self, values: usize) -> Result<(), MaterializationError> {
        self.facet_values = checked_budget_add(
            "materialization change facet values",
            self.facet_values,
            values,
            MAX_MATERIALIZATION_CHANGE_FACET_VALUES,
        )?;
        Ok(())
    }

    fn add_field(
        &mut self,
        resource: &'static str,
        value: &str,
        maximum: usize,
    ) -> Result<(), MaterializationError> {
        if value.len() > maximum {
            return Err(resource_limit(resource, value.len(), maximum));
        }
        self.add_bytes(value.len())?;
        self.add_bytes(MATERIALIZATION_STRING_OVERHEAD_BYTES)
    }
}

fn checked_budget_add(
    resource: &'static str,
    current: usize,
    added: usize,
    maximum: usize,
) -> Result<usize, MaterializationError> {
    let found = current
        .checked_add(added)
        .ok_or_else(|| resource_limit(resource, usize::MAX, maximum))?;
    if found > maximum {
        return Err(resource_limit(resource, found, maximum));
    }
    Ok(found)
}

fn resource_limit(resource: &'static str, found: usize, maximum: usize) -> MaterializationError {
    MaterializationError::ResourceLimit {
        resource,
        found,
        maximum,
    }
}

fn canonical_reference_source_locator_bytes(
    locator: ReferenceSourceLocatorV1,
) -> Result<Vec<u8>, MaterializationError> {
    let bytes = postcard::to_allocvec(&locator)
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    validate_reference_source_locator_bytes(&bytes)?;
    Ok(bytes)
}

fn validate_reference_source_locator_bytes(bytes: &[u8]) -> Result<(), MaterializationError> {
    if bytes.is_empty() || bytes.len() > MAX_MATERIALIZATION_FIELD_BYTES {
        return Err(MaterializationError::InvalidInput(
            "reference source locator bytes are out of bounds".into(),
        ));
    }
    let locator: ReferenceSourceLocatorV1 = postcard::from_bytes(bytes).map_err(|_| {
        MaterializationError::InvalidInput("reference source locator bytes are malformed".into())
    })?;
    let canonical = postcard::to_allocvec(&locator)
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    if canonical != bytes {
        return Err(MaterializationError::InvalidInput(
            "reference source locator bytes are not canonical".into(),
        ));
    }
    Ok(())
}

fn validate_page_name_pair(
    description: &str,
    raw_name: &str,
    normalized_name: &str,
) -> Result<(), MaterializationError> {
    if raw_name.is_empty() || normalized_name.is_empty() {
        return Err(MaterializationError::InvalidInput(format!(
            "{description} has an empty raw/normalized name"
        )));
    }
    if raw_name.len() > MAX_MATERIALIZATION_FIELD_BYTES
        || normalized_name.len() > MAX_MATERIALIZATION_FIELD_BYTES
    {
        return Err(MaterializationError::InvalidInput(format!(
            "{description} name exceeds the materialization field limit"
        )));
    }
    if crate::refs::page_key(raw_name) != normalized_name {
        return Err(MaterializationError::InvalidInput(format!(
            "{description} normalized name does not match refs::page_key"
        )));
    }
    Ok(())
}

fn validate_normalized_page_name(
    description: &str,
    normalized_name: &str,
) -> Result<(), MaterializationError> {
    if normalized_name.is_empty()
        || normalized_name.len() > MAX_MATERIALIZATION_FIELD_BYTES
        || crate::refs::page_key(normalized_name) != normalized_name
    {
        return Err(MaterializationError::InvalidInput(format!(
            "{description} is not a canonical page key"
        )));
    }
    Ok(())
}

fn validate_reference_posting(
    posting: &MaterializedReferencePosting,
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    if !posting.kind.accepts_target(&posting.target) {
        return Err(MaterializationError::InvalidInput(
            "reference kind and target type are incompatible".into(),
        ));
    }
    let locator = canonical_reference_source_locator_bytes(posting.source_locator)?;
    input_budget.add_facet_values(1)?;
    input_budget.add_bytes(REFERENCE_CATALOG_POSTING_OVERHEAD_BYTES)?;
    input_budget.add_bytes(locator.len())?;
    posting.target.validate(input_budget)
}

fn validate_alias_declaration(
    alias: &MaterializedAliasDeclaration,
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    let locator = canonical_reference_source_locator_bytes(alias.source_locator)?;
    validate_page_name_pair(
        "reference alias declaration",
        &alias.raw_alias,
        &alias.normalized_alias,
    )?;
    input_budget.add_facet_values(1)?;
    input_budget.add_bytes(REFERENCE_CATALOG_ALIAS_OVERHEAD_BYTES)?;
    input_budget.add_bytes(locator.len())?;
    input_budget.add_field(
        "reference alias raw bytes",
        &alias.raw_alias,
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    input_budget.add_field(
        "reference alias normalized bytes",
        &alias.normalized_alias,
        MAX_MATERIALIZATION_FIELD_BYTES,
    )
}

fn block_owner_page(state: &super::BlockState) -> Option<PageId> {
    match state.owner {
        BlockOwner::Page(page_id) => Some(page_id),
        BlockOwner::Tombstone => None,
    }
}

fn validate_page(
    page: &MaterializedPageInput,
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    input_budget.add_bytes(MATERIALIZATION_PAGE_OVERHEAD_BYTES)?;
    input_budget.add_field(
        "page name bytes",
        &page.name,
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    input_budget.add_field(
        "page name key bytes",
        &page.name_key,
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    input_budget.add_field(
        "page path bytes",
        page.path.as_str(),
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    if let Some(preamble) = &page.preamble {
        input_budget.add_field(
            "page preamble bytes",
            preamble,
            MAX_MATERIALIZATION_PREAMBLE_BYTES,
        )?;
    }
    input_budget.add_field(
        "page searchable text bytes",
        &page.searchable_text,
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    if page.name.is_empty() || page.name_key.is_empty() {
        return Err(MaterializationError::InvalidInput(format!(
            "page {} has an empty name/name key",
            page.page_id
        )));
    }
    validate_references(&page.references, input_budget)?;
    validate_properties(&page.properties, input_budget)?;
    validate_tags(&page.tags, input_budget)?;
    input_budget.add_blocks(page.blocks.len())?;
    let block_ids = page
        .blocks
        .iter()
        .map(|block| block.block_id)
        .collect::<BTreeSet<_>>();
    if block_ids.len() != page.blocks.len() {
        return Err(MaterializationError::InvalidInput(format!(
            "page {} contains duplicate block identities",
            page.page_id
        )));
    }
    if !page
        .blocks
        .windows(2)
        .all(|pair| (&pair[0].order, pair[0].block_id) < (&pair[1].order, pair[1].block_id))
    {
        return Err(MaterializationError::InvalidInput(format!(
            "page {} blocks are not in canonical order",
            page.page_id
        )));
    }
    for block in &page.blocks {
        input_budget.add_bytes(MATERIALIZATION_BLOCK_OVERHEAD_BYTES)?;
        input_budget.add_field(
            "block order bytes",
            &block.order,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        input_budget.add_field(
            "block content bytes",
            &block.content,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        input_budget.add_field(
            "block searchable text bytes",
            &block.searchable_text,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        if block.order.is_empty() {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has an empty order key",
                block.block_id
            )));
        }
        if block
            .heading_level
            .is_some_and(|level| !(1..=6).contains(&level))
        {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has an invalid heading level",
                block.block_id
            )));
        }
        if block.logseq_uuid.is_some() != block.logseq_identity_origin.is_some() {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has incomplete Logseq identity metadata",
                block.block_id
            )));
        }
        if block
            .parent
            .is_some_and(|parent| !block_ids.contains(&parent))
        {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has a parent outside page {}",
                block.block_id, page.page_id
            )));
        }
        validate_references(&block.references, input_budget)?;
        validate_properties(&block.properties, input_budget)?;
        validate_tags(&block.tags, input_budget)?;
        if let Some(task) = &block.task {
            input_budget.add_field(
                "task marker bytes",
                &task.marker,
                MAX_MATERIALIZATION_FIELD_BYTES,
            )?;
            for (resource, value) in [
                ("task priority bytes", task.priority.as_deref()),
                ("task scheduled bytes", task.scheduled.as_deref()),
                ("task deadline bytes", task.deadline.as_deref()),
            ] {
                if let Some(value) = value {
                    input_budget.add_field(resource, value, MAX_MATERIALIZATION_FIELD_BYTES)?;
                }
            }
            if task.marker.is_empty() {
                return Err(MaterializationError::InvalidInput(format!(
                    "block {} has an empty task marker",
                    block.block_id
                )));
            }
        }
    }
    Ok(())
}

fn validate_references(
    references: &[MaterializedReference],
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    let bytes = references
        .len()
        .checked_mul(MATERIALIZATION_REFERENCE_OVERHEAD_BYTES)
        .ok_or_else(|| {
            resource_limit(
                "reference facet bytes",
                usize::MAX,
                MAX_MATERIALIZATION_FACET_BYTES,
            )
        })?;
    validate_facet("reference", references.len(), bytes)?;
    input_budget.add_facet_values(references.len())?;
    input_budget.add_bytes(bytes)
}

fn validate_properties(
    properties: &[MaterializedProperty],
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    let bytes = properties.iter().try_fold(0_usize, |total, property| {
        total
            .checked_add(property.name.len())
            .and_then(|total| total.checked_add(property.value.len()))
            .and_then(|total| total.checked_add(MATERIALIZATION_PROPERTY_OVERHEAD_BYTES))
    });
    let bytes = bytes.ok_or_else(|| {
        resource_limit(
            "property facet bytes",
            usize::MAX,
            MAX_MATERIALIZATION_FACET_BYTES,
        )
    })?;
    validate_facet("property", properties.len(), bytes)?;
    if properties.iter().any(|property| property.name.is_empty()) {
        return Err(MaterializationError::InvalidInput(
            "property names must be non-empty".into(),
        ));
    }
    input_budget.add_facet_values(properties.len())?;
    for property in properties {
        input_budget.add_bytes(MATERIALIZATION_PROPERTY_OVERHEAD_BYTES)?;
        input_budget.add_field(
            "property name bytes",
            &property.name,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        input_budget.add_field(
            "property value bytes",
            &property.value,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
    }
    Ok(())
}

fn validate_tags(
    tags: &[String],
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    let bytes = tags.iter().try_fold(0_usize, |total, tag| {
        total
            .checked_add(tag.len())
            .and_then(|total| total.checked_add(MATERIALIZATION_TAG_OVERHEAD_BYTES))
    });
    let bytes = bytes.ok_or_else(|| {
        resource_limit(
            "tag facet bytes",
            usize::MAX,
            MAX_MATERIALIZATION_FACET_BYTES,
        )
    })?;
    validate_facet("tag", tags.len(), bytes)?;
    if tags.iter().any(String::is_empty) {
        return Err(MaterializationError::InvalidInput(
            "tags must be non-empty".into(),
        ));
    }
    input_budget.add_facet_values(tags.len())?;
    for tag in tags {
        input_budget.add_bytes(MATERIALIZATION_TAG_OVERHEAD_BYTES)?;
        input_budget.add_field("tag bytes", tag, MAX_MATERIALIZATION_FIELD_BYTES)?;
    }
    Ok(())
}

fn validate_facet(
    facet: &'static str,
    values: usize,
    bytes: usize,
) -> Result<(), MaterializationError> {
    if values > MAX_MATERIALIZATION_FACET_VALUES {
        return Err(resource_limit(
            match facet {
                "reference" => "reference facet values",
                "property" => "property facet values",
                "tag" => "tag facet values",
                _ => "materialization facet values",
            },
            values,
            MAX_MATERIALIZATION_FACET_VALUES,
        ));
    }
    if bytes > MAX_MATERIALIZATION_FACET_BYTES {
        return Err(resource_limit(
            match facet {
                "reference" => "reference facet bytes",
                "property" => "property facet bytes",
                "tag" => "tag facet bytes",
                _ => "materialization facet bytes",
            },
            bytes,
            MAX_MATERIALIZATION_FACET_BYTES,
        ));
    }
    Ok(())
}

fn strictly_sorted_unique_by<T, K: Ord>(values: &[T], key: impl Fn(&T) -> K) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

#[cfg(test)]
pub(crate) fn initialize_schema(
    connection: &Connection,
    empty_frontier_digest: ContentDigest,
) -> Result<(), MaterializationError> {
    storage::initialize_materialization_schema_for_test(connection, empty_frontier_digest)
        .map_err(Into::into)
}

#[cfg(test)]
pub(crate) fn apply_change(
    transaction: &Transaction<'_>,
    change: &MaterializationChange,
    semantic_effect: &[u8],
    sequence: u64,
    input_digest: ContentDigest,
    post_frontier_digest: ContentDigest,
    authenticated_reference: Option<&AuthenticatedReferenceMaterialization>,
) -> Result<ApplyChangeInstrumentation, MaterializationError> {
    let (physical, authenticated) =
        lower_validated_change(change, semantic_effect, authenticated_reference)?;
    storage::apply_materialization_change_for_test(
        transaction,
        &physical,
        sequence,
        input_digest,
        post_frontier_digest,
        authenticated.as_ref(),
    )
    .map_err(Into::into)
}

pub(crate) fn lower_validated_change(
    change: &MaterializationChange,
    semantic_effect: &[u8],
    authenticated_reference: Option<&AuthenticatedReferenceMaterialization>,
) -> Result<
    (
        storage::PhysicalMaterializationChange,
        Option<storage::PhysicalAuthenticatedReference>,
    ),
    MaterializationError,
> {
    change.validate_shape()?;
    let effect = SemanticEffect::decode(semantic_effect)
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    change.validate_against_effect(&effect)?;
    if let Some(reference_catalog) = change.reference_catalog() {
        let authenticated = authenticated_reference.ok_or_else(|| {
            MaterializationError::Incomplete(
                "authenticated reference materialization requires accepted event evidence".into(),
            )
        })?;
        reference_catalog.validate_for_authenticated_transition(authenticated, &effect)?;
    }

    let pages_with_live_metadata_delta = effect
        .pages()
        .iter()
        .filter(|delta| matches!(delta.after.as_ref(), Some(PageState::Live { .. })))
        .map(|delta| delta.page_id.as_uuid().into_bytes())
        .collect();
    let replacements = change
        .replacements
        .iter()
        .map(lower_page)
        .collect::<Result<Vec<_>, _>>()?;
    let reference_catalog = change
        .reference_catalog
        .as_ref()
        .map(lower_reference_catalog)
        .transpose()?;
    let authenticated =
        authenticated_reference.map(|value| storage::PhysicalAuthenticatedReference {
            event_binding_digest: value.event_binding_digest,
            prior_frontier_root_digest: value.prior_frontier_root_digest,
            post_frontier_root_digest: value.post_frontier_root_digest,
        });
    Ok((
        storage::PhysicalMaterializationChange {
            batch_id: change.batch_id.as_uuid().into_bytes(),
            replacements,
            deletions: change
                .deletions
                .iter()
                .map(|page_id| page_id.as_uuid().into_bytes())
                .collect(),
            pages_with_live_metadata_delta,
            reference_catalog,
        },
        authenticated,
    ))
}

fn lower_page(page: &MaterializedPageInput) -> Result<storage::PhysicalPage, MaterializationError> {
    Ok(storage::PhysicalPage {
        page_id: page.page_id.as_uuid().into_bytes(),
        home_document_id: page.home_document_id.as_uuid().into_bytes(),
        name: page.name.clone(),
        name_key: page.name_key.clone(),
        path: page.path.as_str().to_owned(),
        text_kind: text_kind_to_sql(page.kind),
        preamble: page.preamble.clone(),
        searchable_text: page.searchable_text.clone(),
        references: page.references.iter().map(lower_reference).collect(),
        properties: page.properties.iter().map(lower_property).collect(),
        tags: page.tags.clone(),
        blocks: page
            .blocks
            .iter()
            .map(lower_block)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn lower_block(
    block: &MaterializedBlockInput,
) -> Result<storage::PhysicalBlock, MaterializationError> {
    Ok(storage::PhysicalBlock {
        block_id: block.block_id.as_uuid().into_bytes(),
        home_document_id: block.home_document_id.as_uuid().into_bytes(),
        parent: block.parent.map(|id| id.as_uuid().into_bytes()),
        order: block.order.clone(),
        content: block.content.clone(),
        searchable_text: block.searchable_text.clone(),
        heading_level: block.heading_level,
        collapsed: block.collapsed,
        logseq_uuid: block.logseq_uuid.map(|id| id.as_uuid().into_bytes()),
        logseq_identity_origin: block.logseq_identity_origin.map(identity_origin_to_sql),
        references: block.references.iter().map(lower_reference).collect(),
        properties: block.properties.iter().map(lower_property).collect(),
        tags: block.tags.clone(),
        task: block.task.as_ref().map(|task| storage::PhysicalTask {
            marker: task.marker.clone(),
            priority: task.priority.clone(),
            scheduled: task.scheduled.clone(),
            deadline: task.deadline.clone(),
        }),
    })
}

fn lower_reference(reference: &MaterializedReference) -> storage::PhysicalReference {
    storage::PhysicalReference {
        target: lower_entity(reference.target),
        kind: reference.kind.sql_value(),
    }
}

fn lower_property(property: &MaterializedProperty) -> storage::PhysicalProperty {
    storage::PhysicalProperty {
        name: property.name.clone(),
        value: property.value.clone(),
    }
}

fn lower_entity(entity: MaterializedEntityId) -> storage::PhysicalEntityId {
    match entity {
        MaterializedEntityId::Page(id) => {
            storage::PhysicalEntityId::Page(id.as_uuid().into_bytes())
        }
        MaterializedEntityId::Block(id) => {
            storage::PhysicalEntityId::Block(id.as_uuid().into_bytes())
        }
    }
}

fn lower_reference_catalog(
    input: &ReferenceCatalogMaterializationInput,
) -> Result<storage::PhysicalReferenceCatalogChange, MaterializationError> {
    let prior_catalog_root = input
        .prior_catalog_root
        .encode()
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    let prior_catalog_root_digest = input
        .prior_catalog_root
        .external_digest()
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    let post_catalog_root = input
        .post_catalog_root
        .encode()
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    let post_catalog_root_digest = input
        .post_catalog_root
        .external_digest()
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    let extractor_dependency_stamp_digest = ReferenceExtractorDependencyStamp::new(
        input.post_catalog_root.extractor_digest(),
        input.post_catalog_root.policy_digest(),
    )?
    .digest()?;
    let canonical_bytes = postcard::to_allocvec(input)
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    Ok(storage::PhysicalReferenceCatalogChange {
        prior_catalog_root,
        prior_catalog_root_digest,
        prior_source_count: input.prior_catalog_root.source_count(),
        post_catalog_root,
        post_catalog_root_digest,
        post_source_count: input.post_catalog_root.source_count(),
        coverage_digest: input.post_catalog_root.source_coverage_root(),
        extractor_dependency_stamp_digest,
        postings: lower_reference_postings(&input.postings)?,
        aliases: lower_alias_declarations(&input.aliases)?,
        coverage: lower_source_coverage(&input.coverage)?,
        removed_sources: input
            .removed_sources
            .iter()
            .map(|id| id.as_uuid().into_bytes())
            .collect(),
        canonical_bytes,
    })
}

fn lower_reference_postings(
    postings: &[MaterializedReferencePosting],
) -> Result<Vec<storage::PhysicalReferencePosting>, MaterializationError> {
    postings
        .iter()
        .map(|posting| {
            Ok(storage::PhysicalReferencePosting {
                source_page_id: posting.source_page_id.as_uuid().into_bytes(),
                source_entity: lower_entity(posting.source_entity),
                source_locator: canonical_reference_source_locator_bytes(posting.source_locator)?,
                ordinal: posting.ordinal,
                kind: posting.kind.sql_value(),
                target: match &posting.target {
                    MaterializedReferenceTarget::PageName {
                        raw_name,
                        normalized_name,
                        resolved_page_id,
                    } => storage::PhysicalReferenceTarget::PageName {
                        raw_name: raw_name.clone(),
                        normalized_name: normalized_name.clone(),
                        resolved_page_id: resolved_page_id.map(|id| id.as_uuid().into_bytes()),
                    },
                    MaterializedReferenceTarget::ExternalUuid {
                        raw_claim,
                        resolved_block_id,
                    } => storage::PhysicalReferenceTarget::ExternalUuid {
                        raw_claim: raw_claim.as_uuid().into_bytes(),
                        resolved_block_id: resolved_block_id.map(|id| id.as_uuid().into_bytes()),
                    },
                },
            })
        })
        .collect()
}

fn lower_alias_declarations(
    aliases: &[MaterializedAliasDeclaration],
) -> Result<Vec<storage::PhysicalAliasDeclaration>, MaterializationError> {
    aliases
        .iter()
        .map(|alias| {
            Ok(storage::PhysicalAliasDeclaration {
                source_page_id: alias.source_page_id.as_uuid().into_bytes(),
                source_entity: lower_entity(alias.source_entity),
                source_locator: canonical_reference_source_locator_bytes(alias.source_locator)?,
                ordinal: alias.ordinal,
                raw_alias: alias.raw_alias.clone(),
                normalized_alias: alias.normalized_alias.clone(),
            })
        })
        .collect()
}

fn lower_source_coverage(
    coverage: &[SourceCoverageFacet],
) -> Result<Vec<storage::PhysicalSourceCoverage>, MaterializationError> {
    coverage
        .iter()
        .map(|facet| {
            Ok(storage::PhysicalSourceCoverage {
                source_page_id: facet.source_page_id.as_uuid().into_bytes(),
                source_digest: facet.source_digest,
                extractor_dependency_stamp_digest: facet.extractor_dependency_stamp.digest()?,
            })
        })
        .collect()
}

/// One bounded chunk of terminal bootstrap rows, before lowering.
///
/// Terminal construction never replays an intermediate page or reference
/// replacement, so a chunk has no deletions, no removed sources, and no prior
/// catalog transition: the accepted terminal catalog root authenticates every
/// row in it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalMaterializationChunk {
    pub(crate) pages: Vec<MaterializedPageInput>,
    pub(crate) coverage: Vec<SourceCoverageFacet>,
    pub(crate) postings: Vec<MaterializedReferencePosting>,
    pub(crate) aliases: Vec<MaterializedAliasDeclaration>,
}

/// Validate and lower one bounded terminal chunk with the same field rules the
/// per-event lowering applies. The caller separately proves the chunk's pages
/// are exactly the authenticated terminal current-path catalog rows, so there
/// is no per-event semantic effect to validate against here.
pub(crate) fn lower_terminal_chunk(
    mut chunk: TerminalMaterializationChunk,
) -> Result<storage::PhysicalTerminalMaterializationChunk, MaterializationError> {
    chunk.pages.sort_unstable_by_key(|page| page.page_id);
    chunk.coverage.sort_unstable();
    chunk.postings.sort_unstable();
    chunk.aliases.sort_unstable();
    let mut input_budget = MaterializationInputBudget::default();
    input_budget.add_pages(chunk.pages.len())?;
    if !strictly_sorted_unique_by(&chunk.pages, |page| page.page_id) {
        return Err(MaterializationError::InvalidInput(
            "terminal pages are not canonical".into(),
        ));
    }
    let mut block_ids = BTreeSet::new();
    for page in &chunk.pages {
        validate_page(page, &mut input_budget)?;
        for block in &page.blocks {
            if !block_ids.insert(block.block_id) {
                return Err(MaterializationError::InvalidInput(format!(
                    "block {} occurs in multiple terminal pages",
                    block.block_id
                )));
            }
        }
    }
    if !strictly_sorted_unique_by(&chunk.coverage, |facet| facet.source_page_id) {
        return Err(MaterializationError::InvalidInput(
            "terminal reference source coverage is not canonical".into(),
        ));
    }
    if !strictly_sorted_unique_by(&chunk.postings, |posting| {
        (
            posting.source_page_id,
            posting.source_entity,
            posting.source_locator,
            posting.ordinal,
        )
    }) {
        return Err(MaterializationError::InvalidInput(
            "terminal reference postings are not canonical".into(),
        ));
    }
    if !strictly_sorted_unique_by(&chunk.aliases, |alias| {
        (
            alias.source_page_id,
            alias.source_entity,
            alias.source_locator,
            alias.ordinal,
        )
    }) {
        return Err(MaterializationError::InvalidInput(
            "terminal reference alias declarations are not canonical".into(),
        ));
    }
    let covered = chunk
        .coverage
        .iter()
        .map(|facet| facet.source_page_id)
        .collect::<BTreeSet<_>>();
    for posting in &chunk.postings {
        if !covered.contains(&posting.source_page_id) {
            return Err(MaterializationError::InvalidInput(
                "terminal reference posting has no source coverage".into(),
            ));
        }
        validate_reference_posting(posting, &mut input_budget)?;
    }
    for alias in &chunk.aliases {
        if !covered.contains(&alias.source_page_id) {
            return Err(MaterializationError::InvalidInput(
                "terminal reference alias has no source coverage".into(),
            ));
        }
        validate_alias_declaration(alias, &mut input_budget)?;
    }
    for facet in &chunk.coverage {
        facet.extractor_dependency_stamp.validate()?;
        let _ = facet.extractor_dependency_stamp.digest()?;
        input_budget.add_facet_values(1)?;
        input_budget.add_bytes(REFERENCE_CATALOG_COVERAGE_OVERHEAD_BYTES)?;
    }
    Ok(storage::PhysicalTerminalMaterializationChunk {
        pages: chunk
            .pages
            .iter()
            .map(lower_page)
            .collect::<Result<Vec<_>, _>>()?,
        coverage: lower_source_coverage(&chunk.coverage)?,
        postings: lower_reference_postings(&chunk.postings)?,
        aliases: lower_alias_declarations(&chunk.aliases)?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedPageRow {
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub name: String,
    pub name_key: String,
    pub path: ManagedPath,
    pub kind: ManagedTextKind,
    pub preamble: Option<String>,
    pub searchable_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedBlockRow {
    pub block_id: BlockId,
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub parent: Option<BlockId>,
    pub order: String,
    pub content: String,
    pub searchable_text: String,
    pub heading_level: Option<u8>,
    pub collapsed: bool,
    pub logseq_uuid: Option<LogseqUuid>,
    pub logseq_identity_origin: Option<LogseqIdentityOrigin>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedReferrerRow {
    pub source: MaterializedEntityId,
    pub source_page_id: PageId,
    pub kind: MaterializedReferenceKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedPropertyRow {
    pub owner: MaterializedEntityId,
    pub page_id: PageId,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedTagRow {
    pub owner: MaterializedEntityId,
    pub page_id: PageId,
    pub tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedTaskRow {
    pub block_id: BlockId,
    pub page_id: PageId,
    pub marker: String,
    pub priority: Option<String>,
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedSearchHit {
    pub entity: MaterializedEntityId,
    pub page_id: PageId,
    pub text: String,
    pub rank: f64,
}

pub struct SqliteMaterializedRead<'a> {
    inner: storage::SqliteMaterializedRead<'a>,
}

impl<'a> SqliteMaterializedRead<'a> {
    pub(crate) fn from_storage(inner: storage::SqliteMaterializedRead<'a>) -> Self {
        Self { inner }
    }

    #[cfg(test)]
    pub(crate) fn new(
        connection: &'a Connection,
        acceptance_sequence: u64,
        frontier_digest: ContentDigest,
    ) -> Result<Self, MaterializationError> {
        Ok(Self::from_storage(
            storage::SqliteMaterializedRead::from_connection_for_test(
                connection,
                acceptance_sequence,
                frontier_digest,
            )?,
        ))
    }

    pub const fn acceptance_sequence(&self) -> u64 {
        self.inner.acceptance_sequence()
    }

    pub fn page(
        &self,
        page_id: PageId,
    ) -> Result<Option<MaterializedPageRow>, MaterializationError> {
        self.inner
            .page_with_header_validation(
                page_id.as_uuid().into_bytes(),
                validate_storage_page_header,
            )?
            .map(page_row_from_storage)
            .transpose()
    }

    pub fn block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<MaterializedBlockRow>, MaterializationError> {
        self.inner
            .block(block_id.as_uuid().into_bytes())?
            .map(block_row_from_storage)
            .transpose()
    }

    pub fn pages_by_name(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        convert_rows(
            self.inner.pages_by_name_with_header_validation(
                name,
                limit,
                validate_storage_page_header,
            )?,
            page_row_from_storage,
        )
    }

    pub fn pages_by_name_key(
        &self,
        name_key: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        convert_rows(
            self.inner.pages_by_name_key_with_header_validation(
                name_key,
                limit,
                validate_storage_page_header,
            )?,
            page_row_from_storage,
        )
    }

    pub fn pages_by_name_key_and_kind(
        &self,
        name_key: &str,
        kind: ManagedTextKind,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        convert_rows(
            self.inner
                .pages_by_name_key_and_kind_with_header_validation(
                    name_key,
                    text_kind_to_sql(kind),
                    limit,
                    validate_storage_page_header,
                )?,
            page_row_from_storage,
        )
    }

    pub fn pages_by_path(
        &self,
        path: &ManagedPath,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        convert_rows(
            self.inner.pages_by_path_with_header_validation(
                &path.as_str().to_owned(),
                limit,
                validate_storage_page_header,
            )?,
            page_row_from_storage,
        )
    }

    pub fn pages(
        &self,
        kind: Option<ManagedTextKind>,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        convert_rows(
            self.inner.pages_with_header_validation(
                kind.map(text_kind_to_sql),
                limit,
                validate_storage_page_header,
            )?,
            page_row_from_storage,
        )
    }

    pub fn blocks_on_page(
        &self,
        page_id: PageId,
        limit: usize,
    ) -> Result<Vec<MaterializedBlockRow>, MaterializationError> {
        convert_rows(
            self.inner
                .blocks_on_page(page_id.as_uuid().into_bytes(), limit)?,
            block_row_from_storage,
        )
    }

    pub fn referrers_to(
        &self,
        target: MaterializedEntityId,
        limit: usize,
    ) -> Result<Vec<MaterializedReferrerRow>, MaterializationError> {
        convert_rows(
            self.inner.referrers_to(lower_entity(target), limit)?,
            referrer_row_from_storage,
        )
    }

    pub fn properties(
        &self,
        owner: MaterializedEntityId,
        limit: usize,
    ) -> Result<Vec<MaterializedPropertyRow>, MaterializationError> {
        convert_rows(
            self.inner.properties(lower_entity(owner), limit)?,
            property_row_from_storage,
        )
    }

    pub fn properties_named(
        &self,
        name: &str,
        value: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MaterializedPropertyRow>, MaterializationError> {
        convert_rows(
            self.inner.properties_named(name, value, limit)?,
            property_row_from_storage,
        )
    }

    pub fn tags(
        &self,
        tag: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedTagRow>, MaterializationError> {
        convert_rows(self.inner.tags(tag, limit)?, tag_row_from_storage)
    }

    pub fn tasks(
        &self,
        marker: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MaterializedTaskRow>, MaterializationError> {
        convert_rows(self.inner.tasks(marker, limit)?, task_row_from_storage)
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedSearchHit>, MaterializationError> {
        convert_rows(self.inner.search(query, limit)?, search_hit_from_storage)
    }
}

fn convert_rows<T, U>(
    rows: Vec<T>,
    convert: impl Fn(T) -> Result<U, MaterializationError>,
) -> Result<Vec<U>, MaterializationError> {
    rows.into_iter().map(convert).collect()
}

fn validate_storage_page_header(
    path: &str,
    kind: i64,
) -> Result<(), storage::MaterializationError> {
    ManagedPath::parse(path).map_err(|error| {
        storage::MaterializationError::Corrupt(format!("malformed managed path row: {error}"))
    })?;
    text_kind_from_sql(kind)
        .map(|_| ())
        .map_err(|error| storage::MaterializationError::Corrupt(error.to_string()))
}

fn page_row_from_storage(
    row: storage::PhysicalPageRow,
) -> Result<MaterializedPageRow, MaterializationError> {
    Ok(MaterializedPageRow {
        page_id: PageId::from_uuid(Uuid::from_bytes(row.page_id)),
        home_document_id: DocumentId::from_uuid(Uuid::from_bytes(row.home_document_id)),
        name: row.name,
        name_key: row.name_key,
        path: ManagedPath::parse(row.path).map_err(typed_sql_decode_error)?,
        kind: text_kind_from_sql(row.text_kind).map_err(typed_sql_decode_error)?,
        preamble: row.preamble,
        searchable_text: row.searchable_text,
    })
}

fn block_row_from_storage(
    row: storage::PhysicalBlockRow,
) -> Result<MaterializedBlockRow, MaterializationError> {
    Ok(MaterializedBlockRow {
        block_id: BlockId::from_uuid(Uuid::from_bytes(row.block_id)),
        page_id: PageId::from_uuid(Uuid::from_bytes(row.page_id)),
        home_document_id: DocumentId::from_uuid(Uuid::from_bytes(row.home_document_id)),
        parent: row
            .parent
            .map(|id| BlockId::from_uuid(Uuid::from_bytes(id))),
        order: row.order,
        content: row.content,
        searchable_text: row.searchable_text,
        heading_level: row.heading_level,
        collapsed: row.collapsed,
        logseq_uuid: row
            .logseq_uuid
            .map(|id| LogseqUuid::from_uuid(Uuid::from_bytes(id))),
        logseq_identity_origin: row
            .logseq_identity_origin
            .map(identity_origin_from_sql)
            .transpose()
            .map_err(typed_sql_decode_error)?,
    })
}

fn entity_from_storage(entity: storage::PhysicalEntityId) -> MaterializedEntityId {
    match entity {
        storage::PhysicalEntityId::Page(id) => {
            MaterializedEntityId::Page(PageId::from_uuid(Uuid::from_bytes(id)))
        }
        storage::PhysicalEntityId::Block(id) => {
            MaterializedEntityId::Block(BlockId::from_uuid(Uuid::from_bytes(id)))
        }
    }
}

fn referrer_row_from_storage(
    row: storage::PhysicalReferrerRow,
) -> Result<MaterializedReferrerRow, MaterializationError> {
    Ok(MaterializedReferrerRow {
        source: entity_from_storage(row.source),
        source_page_id: PageId::from_uuid(Uuid::from_bytes(row.source_page_id)),
        kind: MaterializedReferenceKind::from_sql(row.kind)?,
    })
}

fn property_row_from_storage(
    row: storage::PhysicalPropertyRow,
) -> Result<MaterializedPropertyRow, MaterializationError> {
    Ok(MaterializedPropertyRow {
        owner: entity_from_storage(row.owner),
        page_id: PageId::from_uuid(Uuid::from_bytes(row.page_id)),
        name: row.name,
        value: row.value,
    })
}

fn tag_row_from_storage(
    row: storage::PhysicalTagRow,
) -> Result<MaterializedTagRow, MaterializationError> {
    Ok(MaterializedTagRow {
        owner: entity_from_storage(row.owner),
        page_id: PageId::from_uuid(Uuid::from_bytes(row.page_id)),
        tag: row.tag,
    })
}

fn task_row_from_storage(
    row: storage::PhysicalTaskRow,
) -> Result<MaterializedTaskRow, MaterializationError> {
    Ok(MaterializedTaskRow {
        block_id: BlockId::from_uuid(Uuid::from_bytes(row.block_id)),
        page_id: PageId::from_uuid(Uuid::from_bytes(row.page_id)),
        marker: row.marker,
        priority: row.priority,
        scheduled: row.scheduled,
        deadline: row.deadline,
    })
}

fn search_hit_from_storage(
    row: storage::PhysicalSearchHit,
) -> Result<MaterializedSearchHit, MaterializationError> {
    Ok(MaterializedSearchHit {
        entity: entity_from_storage(row.entity),
        page_id: PageId::from_uuid(Uuid::from_bytes(row.page_id)),
        text: row.text,
        rank: row.rank,
    })
}

fn typed_sql_decode_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> MaterializationError {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(error))
        .into()
}

fn text_kind_to_sql(kind: ManagedTextKind) -> i64 {
    match kind {
        ManagedTextKind::Page => 0,
        ManagedTextKind::Journal => 1,
    }
}

fn text_kind_from_sql(value: i64) -> Result<ManagedTextKind, MaterializationError> {
    match value {
        0 => Ok(ManagedTextKind::Page),
        1 => Ok(ManagedTextKind::Journal),
        _ => Err(MaterializationError::Corrupt(format!(
            "unknown managed text kind {value}"
        ))),
    }
}

fn identity_origin_to_sql(origin: LogseqIdentityOrigin) -> i64 {
    match origin {
        LogseqIdentityOrigin::ExternalImported => 0,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockReference,
        } => 1,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockEmbed,
        } => 2,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::Export,
        } => 3,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::CopiedDeepLink,
        } => 4,
    }
}

fn identity_origin_from_sql(value: i64) -> Result<LogseqIdentityOrigin, MaterializationError> {
    match value {
        0 => Ok(LogseqIdentityOrigin::ExternalImported),
        1 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockReference,
        }),
        2 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockEmbed,
        }),
        3 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::Export,
        }),
        4 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::CopiedDeepLink,
        }),
        _ => Err(MaterializationError::Corrupt(format!(
            "unknown Logseq identity origin {value}"
        ))),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaterializationError {
    Sqlite(String),
    Schema(String),
    Corrupt(String),
    ResourceLimit {
        resource: &'static str,
        found: usize,
        maximum: usize,
    },
    InvalidInput(String),
    Incomplete(String),
    Contradiction(String),
    BatchMismatch {
        expected: BatchId,
        found: BatchId,
    },
    Stale {
        materialized: u64,
        frontier: u64,
    },
    DuplicateCollision(BatchId),
    InvalidQuery(String),
}

impl fmt::Display for MaterializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite materialization error: {error}"),
            Self::Schema(error) => write!(f, "materialization schema mismatch: {error}"),
            Self::Corrupt(error) => write!(f, "corrupt materialization: {error}"),
            Self::ResourceLimit {
                resource,
                found,
                maximum,
            } => write!(
                f,
                "materialization {resource} {found} exceeds limit {maximum}"
            ),
            Self::InvalidInput(error) => write!(f, "invalid materialization input: {error}"),
            Self::Incomplete(error) => write!(f, "incomplete materialization input: {error}"),
            Self::Contradiction(error) => {
                write!(f, "materialization contradicts accepted semantics: {error}")
            }
            Self::BatchMismatch { expected, found } => {
                write!(
                    f,
                    "materialization batch {found} != accepted batch {expected}"
                )
            }
            Self::Stale {
                materialized,
                frontier,
            } => write!(
                f,
                "materialization frontier {materialized} is stale against accepted frontier {frontier}"
            ),
            Self::DuplicateCollision(batch_id) => {
                write!(
                    f,
                    "materialization for batch {batch_id} has different canonical bytes"
                )
            }
            Self::InvalidQuery(error) => write!(f, "invalid materialization query: {error}"),
        }
    }
}

impl std::error::Error for MaterializationError {}

impl From<rusqlite::Error> for MaterializationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error.to_string())
    }
}

impl From<storage::MaterializationError> for MaterializationError {
    fn from(error: storage::MaterializationError) -> Self {
        match error {
            storage::MaterializationError::Sqlite(error) => Self::Sqlite(error),
            storage::MaterializationError::Schema(error) => Self::Schema(error),
            storage::MaterializationError::Corrupt(error) => Self::Corrupt(error),
            storage::MaterializationError::ResourceLimit {
                resource,
                found,
                maximum,
            } => Self::ResourceLimit {
                resource,
                found,
                maximum,
            },
            storage::MaterializationError::InvalidInput(error) => Self::InvalidInput(error),
            storage::MaterializationError::Incomplete(error) => Self::Incomplete(error),
            storage::MaterializationError::Contradiction(error) => Self::Contradiction(error),
            storage::MaterializationError::Stale {
                materialized,
                frontier,
            } => Self::Stale {
                materialized,
                frontier,
            },
            storage::MaterializationError::InvalidQuery(error) => Self::InvalidQuery(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_id(value: u128) -> PageId {
        PageId::from_uuid(Uuid::from_u128(value))
    }

    fn document_id(value: u128) -> DocumentId {
        DocumentId::from_uuid(Uuid::from_u128(value))
    }

    fn block_id(value: u128) -> BlockId {
        BlockId::from_uuid(Uuid::from_u128(value))
    }

    fn batch_id(value: u128) -> BatchId {
        BatchId::from_uuid(Uuid::from_u128(value))
    }

    fn extractor_stamp() -> ReferenceExtractorDependencyStamp {
        ReferenceExtractorDependencyStamp::new(
            ContentDigest::of(b"test extractor"),
            ContentDigest::of(b"test policy"),
        )
        .unwrap()
    }

    fn empty_reference_catalog_root() -> ReferenceCatalogRootV2 {
        let page_names = super::super::PageNameOwnershipRootV1::empty();
        ReferenceCatalogRootV2::empty(
            &super::super::ReferenceCatalogPolicyV1::default(),
            &page_names,
            ContentDigest::of(b"external UUID authority"),
        )
        .unwrap()
    }

    #[test]
    fn reference_catalog_input_preserves_raw_spellings_uuid_claims_and_structural_locators() {
        let source_page = page_id(1);
        let source_block = block_id(2);
        let locator = ReferenceSourceLocatorV1::Block {
            block_id: source_block,
            home_document_id: document_id(3),
        };
        let locator_bytes = canonical_reference_source_locator_bytes(locator).unwrap();
        assert!(ManagedPath::parse("nested/physical/layout/source.md").is_ok());
        assert!(!locator_bytes
            .windows(b"nested/physical/layout/source.md".len())
            .any(|window| window == b"nested/physical/layout/source.md"));

        let raw_name = " /Über/ ".to_owned();
        let normalized_name = crate::refs::page_key(&raw_name);
        let raw_alias = " /Alias/ ".to_owned();
        let normalized_alias = crate::refs::page_key(&raw_alias);
        let uuid_claim = LogseqUuid::from_uuid(Uuid::from_u128(4));
        let catalog_root = ContentDigest::of(b"catalog root");
        let root = empty_reference_catalog_root();
        let input = ReferenceCatalogMaterializationInput::new(
            root.clone(),
            root,
            vec![MaterializedReferencePosting {
                source_page_id: source_page,
                source_entity: MaterializedEntityId::Block(source_block),
                source_locator: locator,
                ordinal: 0,
                kind: ReferenceCatalogReferenceKind::PropertyKeyPseudoPage,
                target: MaterializedReferenceTarget::PageName {
                    raw_name: raw_name.clone(),
                    normalized_name: normalized_name.clone(),
                    resolved_page_id: None,
                },
            }],
            vec![MaterializedAliasDeclaration {
                source_page_id: source_page,
                source_entity: MaterializedEntityId::Block(source_block),
                source_locator: locator,
                ordinal: 1,
                raw_alias: raw_alias.clone(),
                normalized_alias: normalized_alias.clone(),
            }],
            vec![MaterializedReferenceNameBinding {
                raw_name: raw_name.clone(),
                normalized_name,
                candidate_ordinal: 0,
                resolved_page_id: None,
            }],
            vec![MaterializedReferenceUuidBinding {
                raw_uuid_claim: uuid_claim,
                candidate_ordinal: 0,
                resolved_block_id: None,
            }],
            vec![MaterializedReferenceAliasBinding {
                normalized_alias,
                candidate_ordinal: 0,
                resolved_page_id: None,
                catalog_root_digest: catalog_root,
            }],
            vec![SourceCoverageFacet {
                source_page_id: source_page,
                source_digest: ContentDigest::of(b"source"),
                extractor_dependency_stamp: extractor_stamp(),
            }],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            input.postings[0].target,
            MaterializedReferenceTarget::PageName {
                raw_name,
                normalized_name: crate::refs::page_key(" /Über/ "),
                resolved_page_id: None,
            }
        );
        assert_eq!(input.aliases[0].raw_alias, raw_alias);
        assert_eq!(input.uuid_bindings[0].raw_uuid_claim, uuid_claim);
    }

    #[test]
    fn reference_catalog_input_rejects_malformed_names_locators_and_aggregate_limits() {
        assert!(matches!(
            validate_reference_source_locator_bytes(b"not-a-postcard-locator"),
            Err(MaterializationError::InvalidInput(_))
        ));
        let malformed = MaterializedReferencePosting {
            source_page_id: page_id(1),
            source_entity: MaterializedEntityId::Page(page_id(1)),
            source_locator: ReferenceSourceLocatorV1::Preamble,
            ordinal: 0,
            kind: ReferenceCatalogReferenceKind::PageLink,
            target: MaterializedReferenceTarget::PageName {
                raw_name: "Correct spelling".into(),
                normalized_name: "wrong key".into(),
                resolved_page_id: None,
            },
        };
        assert!(matches!(
            ReferenceCatalogMaterializationInput::new(
                empty_reference_catalog_root(),
                empty_reference_catalog_root(),
                vec![malformed],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(MaterializationError::InvalidInput(_))
        ));

        let mut budget = MaterializationInputBudget::default();
        assert!(matches!(
            budget.add_facet_values(MAX_MATERIALIZATION_CHANGE_FACET_VALUES + 1),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization change facet values",
                ..
            })
        ));
        let mut budget = MaterializationInputBudget::default();
        assert!(matches!(
            budget.add_bytes(MAX_MATERIALIZATION_CHANGE_BYTES + 1),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization change bytes",
                ..
            })
        ));
    }

    #[test]
    fn reference_catalog_input_rejects_cross_kind_target_pairs() {
        for posting in [
            MaterializedReferencePosting {
                source_page_id: page_id(1),
                source_entity: MaterializedEntityId::Page(page_id(1)),
                source_locator: ReferenceSourceLocatorV1::Preamble,
                ordinal: 0,
                kind: ReferenceCatalogReferenceKind::PageLink,
                target: MaterializedReferenceTarget::ExternalUuid {
                    raw_claim: LogseqUuid::from_uuid(Uuid::from_u128(1)),
                    resolved_block_id: None,
                },
            },
            MaterializedReferencePosting {
                source_page_id: page_id(1),
                source_entity: MaterializedEntityId::Page(page_id(1)),
                source_locator: ReferenceSourceLocatorV1::Preamble,
                ordinal: 1,
                kind: ReferenceCatalogReferenceKind::BlockReference,
                target: MaterializedReferenceTarget::PageName {
                    raw_name: "page name".into(),
                    normalized_name: "page name".into(),
                    resolved_page_id: None,
                },
            },
        ] {
            assert!(matches!(
                ReferenceCatalogMaterializationInput::new(
                    empty_reference_catalog_root(),
                    empty_reference_catalog_root(),
                    vec![posting],
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                Err(MaterializationError::InvalidInput(_))
            ));
        }
    }

    fn page_input(page: PageId, searchable_text: String) -> MaterializedPageInput {
        MaterializedPageInput {
            page_id: page,
            home_document_id: document_id(10_000),
            name: "shared".into(),
            name_key: "shared".into(),
            path: ManagedPath::parse(format!("test/{page}.md")).unwrap(),
            kind: ManagedTextKind::Page,
            preamble: None,
            searchable_text,
            references: Vec::new(),
            properties: Vec::new(),
            tags: Vec::new(),
            blocks: Vec::new(),
        }
    }

    fn semantic_effect_for_replacements(pages: &[MaterializedPageInput]) -> Vec<u8> {
        SemanticEffect::new(
            pages
                .iter()
                .map(|page| super::super::PageDelta {
                    page_id: page.page_id,
                    before: None,
                    after: Some(PageState::Live {
                        name: super::super::LogicalPageName::parse(&page.name).unwrap(),
                        path: page.path.clone(),
                        home_document_id: page.home_document_id,
                        kind: page.kind,
                    }),
                })
                .collect(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
        .encode()
        .unwrap()
    }

    #[test]
    fn membership_validation_retains_presence_and_absence_semantics_across_many_blocks() {
        let page_id = page_id(410_000);
        let home_document_id = document_id(410_001);
        let target_block_id = BlockId::from_uuid(Uuid::from_u128(410_020));
        let mut page = page_input(page_id, "membership index".into());
        page.blocks = (0..32)
            .map(|index| {
                let block_id = if index == 23 {
                    target_block_id
                } else {
                    BlockId::from_uuid(Uuid::from_u128(410_100 + index))
                };
                MaterializedBlockInput {
                    block_id,
                    home_document_id,
                    parent: None,
                    order: format!("{index:02}"),
                    content: format!("block {index}"),
                    searchable_text: format!("block {index}"),
                    heading_level: None,
                    collapsed: false,
                    logseq_uuid: None,
                    logseq_identity_origin: None,
                    references: Vec::new(),
                    properties: Vec::new(),
                    tags: Vec::new(),
                    task: None,
                }
            })
            .collect();
        let semantic_effect = SemanticEffect::new(
            Vec::new(),
            Vec::new(),
            vec![super::super::MembershipDelta {
                page_id,
                block_id: target_block_id,
                before: None,
                after: Some(
                    super::super::MembershipClaim::new(home_document_id, None, "23").unwrap(),
                ),
            }],
        )
        .unwrap()
        .encode()
        .unwrap();

        let accepted =
            MaterializationChange::new(batch_id(410_002), vec![page.clone()], Vec::new()).unwrap();
        assert!(accepted
            .validate_against_stored(batch_id(410_002), &semantic_effect)
            .is_ok());

        page.blocks
            .retain(|block| block.block_id != target_block_id);
        let missing =
            MaterializationChange::new(batch_id(410_002), vec![page], Vec::new()).unwrap();
        assert!(matches!(
            missing.validate_against_stored(batch_id(410_002), &semantic_effect),
            Err(MaterializationError::Contradiction(message))
                if message.contains("accepted member") && message.contains("absent")
        ));
    }

    fn resource_limit(error: Result<MaterializationChange, MaterializationError>, resource: &str) {
        assert!(matches!(
            error,
            Err(MaterializationError::ResourceLimit {
                resource: found,
                ..
            }) if found == resource
        ));
    }

    #[test]
    fn materialization_input_limits_reject_before_digest_or_sqlite_write() {
        let page = page_id(1);
        let mut oversized_field = page_input(page, String::new());
        oversized_field.name = "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES + 1);
        resource_limit(
            MaterializationChange::new(batch_id(1), vec![oversized_field], Vec::new()),
            "page name bytes",
        );

        let reference = MaterializedReference {
            target: MaterializedEntityId::Page(page_id(2)),
            kind: MaterializedReferenceKind::Reference,
        };
        let mut oversized_facet_count = page_input(page_id(3), String::new());
        oversized_facet_count.references = vec![reference; MAX_MATERIALIZATION_FACET_VALUES + 1];
        resource_limit(
            MaterializationChange::new(batch_id(2), vec![oversized_facet_count], Vec::new()),
            "reference facet values",
        );

        let oversized_property = MaterializedProperty {
            name: "n".into(),
            value: "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES),
        };
        let mut oversized_facet_bytes = page_input(page_id(4), String::new());
        oversized_facet_bytes.properties = vec![
            oversized_property;
            MAX_MATERIALIZATION_FACET_BYTES
                / MAX_MATERIALIZATION_FIELD_BYTES
        ];
        resource_limit(
            MaterializationChange::new(batch_id(3), vec![oversized_facet_bytes], Vec::new()),
            "property facet bytes",
        );

        let too_many_deletions = (0..=MAX_MATERIALIZATION_CHANGE_PAGES)
            .map(|index| page_id(100_000 + index as u128))
            .collect();
        resource_limit(
            MaterializationChange::new(batch_id(4), Vec::new(), too_many_deletions),
            "materialization change pages",
        );

        let oversized_change = (0..=MAX_MATERIALIZATION_CHANGE_BYTES
            / MAX_MATERIALIZATION_FIELD_BYTES)
            .map(|index| {
                page_input(
                    page_id(200_000 + index as u128),
                    "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES),
                )
            })
            .collect();
        resource_limit(
            MaterializationChange::new(batch_id(5), oversized_change, Vec::new()),
            "materialization change bytes",
        );

        let connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, ContentDigest::of(b"empty")).unwrap();
        let too_large = "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES + 1);
        assert!(connection
            .execute(
                "INSERT INTO pages (
                     page_id, home_document_id, name, name_key, path, text_kind,
                     preamble, searchable_text
                 ) VALUES (?1, ?2, ?3, 'key', 'test/schema.md', 0, NULL, '')",
                params![
                    page_id(300_000).as_uuid().as_bytes().as_slice(),
                    document_id(300_001).as_uuid().as_bytes().as_slice(),
                    too_large,
                ],
            )
            .is_err());
    }

    #[test]
    fn materialization_input_schema_refuses_prior_and_future_before_sqlite_write() {
        assert_eq!(MATERIALIZATION_INPUT_SCHEMA_VERSION, 4);
        let current = MaterializationChange::new(
            batch_id(500_000),
            vec![page_input(page_id(500_001), "current".into())],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(current.schema_version, MATERIALIZATION_INPUT_SCHEMA_VERSION);

        for schema_version in [
            MATERIALIZATION_INPUT_SCHEMA_VERSION - 1,
            MATERIALIZATION_INPUT_SCHEMA_VERSION + 1,
        ] {
            let mut rejected = current.clone();
            rejected.schema_version = schema_version;
            let encoded = postcard::to_allocvec(&rejected).unwrap();
            let rejected: MaterializationChange = postcard::from_bytes(&encoded).unwrap();
            assert!(matches!(
                rejected.digest(),
                Err(MaterializationError::InvalidInput(message))
                    if message == format!("unknown materialization input schema {schema_version}")
            ));

            let mut connection = Connection::open_in_memory().unwrap();
            let empty_frontier = ContentDigest::of(b"empty");
            initialize_schema(&connection, empty_frontier).unwrap();
            let transaction = connection.transaction().unwrap();
            assert!(matches!(
                apply_change(
                    &transaction,
                    &rejected,
                    b"",
                    1,
                    ContentDigest::of(b"input"),
                    ContentDigest::of(b"next"),
                    None,
                ),
                Err(MaterializationError::InvalidInput(message))
                    if message == format!("unknown materialization input schema {schema_version}")
            ));
            transaction.commit().unwrap();
            let page_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
                .unwrap();
            let batch_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM materialization_batches", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let stamp_sequence: i64 = connection
                .query_row(
                    "SELECT acceptance_sequence FROM materialization_stamp WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!((page_count, batch_count, stamp_sequence), (0, 0, 0));
        }
    }

    #[test]
    fn non_page_effect_replacement_without_prior_metadata_fails_closed() {
        let page_id = page_id(600_000);
        let mut page = page_input(page_id, "preamble searchable".into());
        page.preamble = Some("updated preamble".into());
        let change = MaterializationChange::new(batch_id(600_001), vec![page], Vec::new()).unwrap();
        let semantic_effect = SemanticEffect::new_with_page_preambles(
            Vec::new(),
            vec![super::super::PagePreambleDelta {
                page_id,
                home_document_id: document_id(10_000),
                before: Some(super::super::PagePreambleState {
                    page_id,
                    home_document_id: document_id(10_000),
                    preamble: None,
                }),
                after: Some(super::super::PagePreambleState {
                    page_id,
                    home_document_id: document_id(10_000),
                    preamble: Some("updated preamble".into()),
                }),
            }],
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
        .encode()
        .unwrap();
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, ContentDigest::of(b"empty")).unwrap();
        let transaction = connection.transaction().unwrap();
        assert!(matches!(
            apply_change(
                &transaction,
                &change,
                &semantic_effect,
                1,
                change.digest().unwrap(),
                ContentDigest::of(b"next"),
                None,
            ),
            Err(MaterializationError::Incomplete(message))
                if message.contains("lacks prior validated metadata")
        ));
        transaction.commit().unwrap();
        let state: (i64, i64, i64) = connection
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM pages),
                     (SELECT COUNT(*) FROM materialization_batches),
                     (SELECT acceptance_sequence FROM materialization_stamp WHERE singleton = 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, (0, 0, 0));
    }

    #[test]
    fn materialized_reads_reject_oversized_queries_and_aggregate_output() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, ContentDigest::of(b"empty")).unwrap();
        let searchable_text = format!("needle {}", "x".repeat(1024 * 1024 - "needle ".len()));
        let pages_per_change = 33;
        let mut final_frontier = ContentDigest::of(b"empty");
        for group in 0..2 {
            let replacements = (0..pages_per_change)
                .map(|index| {
                    page_input(
                        page_id(400_000 + (group * pages_per_change + index) as u128),
                        searchable_text.clone(),
                    )
                })
                .collect();
            let change = MaterializationChange::new(
                batch_id(400_000 + group as u128),
                replacements,
                Vec::new(),
            )
            .unwrap();
            let digest = change.digest().unwrap();
            final_frontier = ContentDigest::of(&[group as u8 + 1]);
            let semantic_effect = semantic_effect_for_replacements(change.replacements());
            let transaction = connection.transaction().unwrap();
            apply_change(
                &transaction,
                &change,
                &semantic_effect,
                group as u64 + 1,
                digest,
                final_frontier,
                None,
            )
            .unwrap();
            transaction.commit().unwrap();
        }
        let read = SqliteMaterializedRead::new(&connection, 2, final_frontier).unwrap();
        let oversized_query = "q".repeat(MAX_MATERIALIZATION_QUERY_BYTES + 1);
        assert!(matches!(
            read.search(&oversized_query, 1),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization query bytes",
                ..
            })
        ));
        assert!(matches!(
            read.search("needle", pages_per_change * 2),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization read output bytes",
                ..
            })
        ));
    }

    #[test]
    fn malformed_page_path_precedes_aggregate_read_budget_exhaustion() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, ContentDigest::of(b"empty")).unwrap();
        let searchable_text = format!("needle {}", "x".repeat(1024 * 1024 - "needle ".len()));
        let pages_per_change = 33;
        let mut final_frontier = ContentDigest::of(b"empty");
        for group in 0..2 {
            let replacements = (0..pages_per_change)
                .map(|index| {
                    page_input(
                        page_id(700_000 + (group * pages_per_change + index) as u128),
                        searchable_text.clone(),
                    )
                })
                .collect();
            let change = MaterializationChange::new(
                batch_id(700_000 + group as u128),
                replacements,
                Vec::new(),
            )
            .unwrap();
            let digest = change.digest().unwrap();
            final_frontier = ContentDigest::of(&[group as u8 + 1]);
            let semantic_effect = semantic_effect_for_replacements(change.replacements());
            let transaction = connection.transaction().unwrap();
            apply_change(
                &transaction,
                &change,
                &semantic_effect,
                group as u64 + 1,
                digest,
                final_frontier,
                None,
            )
            .unwrap();
            transaction.commit().unwrap();
        }

        connection
            .execute(
                "UPDATE pages SET path = ?1 WHERE page_id = ?2",
                params![
                    "../corrupt.md",
                    page_id(700_000).as_uuid().as_bytes().as_slice(),
                ],
            )
            .unwrap();

        let read = SqliteMaterializedRead::new(&connection, 2, final_frontier).unwrap();
        assert!(matches!(
            read.pages(None, pages_per_change * 2),
            Err(MaterializationError::Corrupt(message))
                if message.contains("malformed managed path row")
        ));
    }
}
