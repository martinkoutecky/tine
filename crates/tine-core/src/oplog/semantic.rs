use std::fmt;

use serde::{Deserialize, Serialize};

use super::{BlockId, DocumentId, LogseqUuid, ManagedPath, PageId};

pub const SEMANTIC_EFFECT_SCHEMA_VERSION: u32 = 3;
pub const MAX_SEMANTIC_EFFECT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SEMANTIC_DELTA_ENTRIES: usize = 100_000;
pub const MAX_BLOCK_CONTENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PAGE_PREAMBLE_BYTES: usize = 16 * 1024 * 1024;
const SEMANTIC_MAGIC: &[u8; 8] = b"TINESEM1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageState {
    Live {
        path: ManagedPath,
        home_document_id: DocumentId,
    },
    Tombstone {
        home_document_id: DocumentId,
    },
}

impl PageState {
    pub const fn home_document_id(&self) -> DocumentId {
        match self {
            Self::Live {
                home_document_id, ..
            }
            | Self::Tombstone { home_document_id } => *home_document_id,
        }
    }

    pub fn path(&self) -> Option<&ManagedPath> {
        match self {
            Self::Live { path, .. } => Some(path),
            Self::Tombstone { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockOwner {
    Page(PageId),
    Tombstone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyGeneratedAnchorReason {
    BlockReference,
    BlockEmbed,
    Export,
    CopiedDeepLink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogseqIdentityOrigin {
    ExternalImported,
    PolicyGenerated { reason: PolicyGeneratedAnchorReason },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockState {
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
    pub owner: BlockOwner,
    pub logseq_uuid: Option<LogseqUuid>,
    pub logseq_identity_origin: Option<LogseqIdentityOrigin>,
    pub content: String,
}

impl BlockState {
    pub const fn policy_generated_logseq_uuid(&self) -> Option<LogseqUuid> {
        match (self.logseq_uuid, self.logseq_identity_origin) {
            (Some(uuid), Some(LogseqIdentityOrigin::PolicyGenerated { .. })) => Some(uuid),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PagePreambleState {
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub preamble: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipClaim {
    pub home_document_id: DocumentId,
    pub parent: Option<BlockId>,
    pub order: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibleMembership {
    pub page_id: PageId,
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
    pub parent: Option<BlockId>,
    pub order: String,
}

impl MembershipClaim {
    pub fn new(
        home_document_id: DocumentId,
        parent: Option<BlockId>,
        order: impl Into<String>,
    ) -> Result<Self, SemanticError> {
        let claim = Self {
            home_document_id,
            parent,
            order: order.into(),
        };
        claim.validate()?;
        Ok(claim)
    }

    pub(crate) fn validate(&self) -> Result<(), SemanticError> {
        if self.order.is_empty()
            || self.order.len() > 512
            || self.order.chars().any(char::is_control)
        {
            return Err(SemanticError::InvalidOrderKey);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageDelta {
    pub page_id: PageId,
    pub before: Option<PageState>,
    pub after: Option<PageState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PagePreambleDelta {
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub before: Option<PagePreambleState>,
    pub after: Option<PagePreambleState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDelta {
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
    pub before: Option<BlockState>,
    pub after: Option<BlockState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipDelta {
    pub page_id: PageId,
    pub block_id: BlockId,
    pub before: Option<MembershipClaim>,
    pub after: Option<MembershipClaim>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticEffect {
    semantic_effect_schema_version: u32,
    pages: Vec<PageDelta>,
    page_preambles: Vec<PagePreambleDelta>,
    blocks: Vec<BlockDelta>,
    memberships: Vec<MembershipDelta>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticEffectWire {
    semantic_effect_schema_version: u32,
    pages: Vec<PageDelta>,
    page_preambles: Vec<PagePreambleDelta>,
    blocks: Vec<BlockDelta>,
    memberships: Vec<MembershipDelta>,
}

impl SemanticEffect {
    pub fn new(
        pages: Vec<PageDelta>,
        blocks: Vec<BlockDelta>,
        memberships: Vec<MembershipDelta>,
    ) -> Result<Self, SemanticError> {
        Self::new_with_page_preambles(pages, Vec::new(), blocks, memberships)
    }

    pub fn new_with_page_preambles(
        mut pages: Vec<PageDelta>,
        mut page_preambles: Vec<PagePreambleDelta>,
        mut blocks: Vec<BlockDelta>,
        mut memberships: Vec<MembershipDelta>,
    ) -> Result<Self, SemanticError> {
        pages.sort_unstable_by_key(|delta| delta.page_id);
        page_preambles.sort_unstable_by_key(|delta| (delta.home_document_id, delta.page_id));
        blocks.sort_unstable_by_key(|delta| (delta.home_document_id, delta.block_id));
        memberships.sort_unstable_by_key(|delta| (delta.page_id, delta.block_id));
        let effect = Self {
            semantic_effect_schema_version: SEMANTIC_EFFECT_SCHEMA_VERSION,
            pages,
            page_preambles,
            blocks,
            memberships,
        };
        effect.validate()?;
        Ok(effect)
    }

    pub fn pages(&self) -> &[PageDelta] {
        &self.pages
    }

    pub fn page_preambles(&self) -> &[PagePreambleDelta] {
        &self.page_preambles
    }

    pub fn blocks(&self) -> &[BlockDelta] {
        &self.blocks
    }

    pub fn memberships(&self) -> &[MembershipDelta] {
        &self.memberships
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
            && self.page_preambles.is_empty()
            && self.blocks.is_empty()
            && self.memberships.is_empty()
    }

    pub fn encode(&self) -> Result<Vec<u8>, SemanticError> {
        self.validate()?;
        self.encode_validated()
    }

    fn encode_validated(&self) -> Result<Vec<u8>, SemanticError> {
        let body = postcard::to_allocvec(self)
            .map_err(|error| SemanticError::Encode(error.to_string()))?;
        let mut bytes = Vec::with_capacity(SEMANTIC_MAGIC.len() + body.len());
        bytes.extend_from_slice(SEMANTIC_MAGIC);
        bytes.extend_from_slice(&body);
        if bytes.len() > MAX_SEMANTIC_EFFECT_BYTES {
            return Err(SemanticError::TooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SemanticError> {
        if bytes.len() > MAX_SEMANTIC_EFFECT_BYTES {
            return Err(SemanticError::TooLarge(bytes.len()));
        }
        let Some(body) = bytes.strip_prefix(SEMANTIC_MAGIC) else {
            return Err(SemanticError::Decode(
                "invalid semantic effect magic".into(),
            ));
        };
        let wire: SemanticEffectWire =
            postcard::from_bytes(body).map_err(|error| SemanticError::Decode(error.to_string()))?;
        let effect = Self {
            semantic_effect_schema_version: wire.semantic_effect_schema_version,
            pages: wire.pages,
            page_preambles: wire.page_preambles,
            blocks: wire.blocks,
            memberships: wire.memberships,
        };
        effect.validate()?;
        if effect.encode_validated()?.as_slice() != bytes {
            return Err(SemanticError::NonCanonical);
        }
        Ok(effect)
    }

    fn validate(&self) -> Result<(), SemanticError> {
        if self.semantic_effect_schema_version != SEMANTIC_EFFECT_SCHEMA_VERSION {
            return Err(SemanticError::UnknownVersion(
                self.semantic_effect_schema_version,
            ));
        }
        let entries = self
            .pages
            .len()
            .checked_add(self.page_preambles.len())
            .and_then(|value| value.checked_add(self.blocks.len()))
            .and_then(|value| value.checked_add(self.memberships.len()))
            .ok_or(SemanticError::TooManyEntries(usize::MAX))?;
        if entries > MAX_SEMANTIC_DELTA_ENTRIES {
            return Err(SemanticError::TooManyEntries(entries));
        }
        if !strictly_sorted_by(&self.pages, |delta| delta.page_id)
            || !strictly_sorted_by(&self.page_preambles, |delta| {
                (delta.home_document_id, delta.page_id)
            })
            || !strictly_sorted_by(&self.blocks, |delta| {
                (delta.home_document_id, delta.block_id)
            })
            || !strictly_sorted_by(&self.memberships, |delta| (delta.page_id, delta.block_id))
        {
            return Err(SemanticError::NonCanonical);
        }
        for delta in &self.pages {
            if delta.before == delta.after {
                return Err(SemanticError::UnchangedDelta);
            }
            if let (Some(before), Some(after)) = (&delta.before, &delta.after) {
                if before.home_document_id() != after.home_document_id() {
                    return Err(SemanticError::HomeShardChanged);
                }
            }
        }
        for delta in &self.page_preambles {
            if delta.before == delta.after {
                return Err(SemanticError::UnchangedDelta);
            }
            if delta.before.is_some() && delta.after.is_none() {
                return Err(SemanticError::PagePreambleStateRemoved);
            }
            for state in [&delta.before, &delta.after].into_iter().flatten() {
                if state.page_id != delta.page_id
                    || state.home_document_id != delta.home_document_id
                {
                    return Err(SemanticError::HomeShardChanged);
                }
                if let Some(preamble) = &state.preamble {
                    if preamble.len() > MAX_PAGE_PREAMBLE_BYTES {
                        return Err(SemanticError::PagePreambleTooLarge(preamble.len()));
                    }
                }
            }
        }
        for delta in &self.blocks {
            if delta.before == delta.after {
                return Err(SemanticError::UnchangedDelta);
            }
            if delta.before.is_some() && delta.after.is_none() {
                return Err(SemanticError::BlockStateRemoved);
            }
            for state in [&delta.before, &delta.after].into_iter().flatten() {
                if state.block_id != delta.block_id
                    || state.home_document_id != delta.home_document_id
                {
                    return Err(SemanticError::HomeShardChanged);
                }
                if state.content.len() > MAX_BLOCK_CONTENT_BYTES {
                    return Err(SemanticError::ContentTooLarge(state.content.len()));
                }
                if state.logseq_uuid.is_some() != state.logseq_identity_origin.is_some() {
                    return Err(SemanticError::InvalidLogseqIdentityState);
                }
            }
        }
        for delta in &self.memberships {
            if delta.before == delta.after {
                return Err(SemanticError::UnchangedDelta);
            }
            for claim in [&delta.before, &delta.after].into_iter().flatten() {
                claim.validate()?;
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for SemanticEffect {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SemanticEffectWire::deserialize(deserializer)?;
        let effect = Self {
            semantic_effect_schema_version: wire.semantic_effect_schema_version,
            pages: wire.pages,
            page_preambles: wire.page_preambles,
            blocks: wire.blocks,
            memberships: wire.memberships,
        };
        effect.validate().map_err(serde::de::Error::custom)?;
        Ok(effect)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalSnapshot {
    pub pages: Vec<(PageId, PageState)>,
    pub page_preambles: Vec<PagePreambleState>,
    pub blocks: Vec<BlockState>,
    pub memberships: Vec<VisibleMembership>,
    pub path_conflicts: Vec<(ManagedPath, Vec<PageId>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticError {
    Decode(String),
    Encode(String),
    UnknownVersion(u32),
    TooLarge(usize),
    TooManyEntries(usize),
    ContentTooLarge(usize),
    PagePreambleTooLarge(usize),
    InvalidOrderKey,
    InvalidLogseqIdentityState,
    NonCanonical,
    UnchangedDelta,
    BlockStateRemoved,
    PagePreambleStateRemoved,
    HomeShardChanged,
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "semantic effect decode failed: {error}"),
            Self::Encode(error) => write!(f, "semantic effect encode failed: {error}"),
            Self::UnknownVersion(found) => write!(
                f,
                "unknown semantic effect schema {found}; expected {SEMANTIC_EFFECT_SCHEMA_VERSION}"
            ),
            Self::TooLarge(bytes) => write!(f, "semantic effect is too large: {bytes} bytes"),
            Self::TooManyEntries(entries) => {
                write!(f, "semantic effect has too many entries: {entries}")
            }
            Self::ContentTooLarge(bytes) => write!(f, "block content is too large: {bytes} bytes"),
            Self::PagePreambleTooLarge(bytes) => {
                write!(f, "page preamble is too large: {bytes} bytes")
            }
            Self::InvalidOrderKey => f.write_str("invalid membership order key"),
            Self::InvalidLogseqIdentityState => {
                f.write_str("Logseq UUID and identity origin must be present together")
            }
            Self::NonCanonical => f.write_str("semantic effect is not canonically ordered/encoded"),
            Self::UnchangedDelta => f.write_str("semantic effect contains an unchanged delta"),
            Self::BlockStateRemoved => {
                f.write_str("authoritative block state cannot be physically removed")
            }
            Self::PagePreambleStateRemoved => {
                f.write_str("authoritative page preamble state cannot be physically removed")
            }
            Self::HomeShardChanged => f.write_str("stable home shard identity changed"),
        }
    }
}

impl std::error::Error for SemanticError {}

fn strictly_sorted_by<T, K: Ord>(values: &[T], key: impl Fn(&T) -> K) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}
