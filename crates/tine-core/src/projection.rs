//! Pure document identity and DTO projection.

use crate::doc::{self, DocBlock};
use crate::model::{BlockDto, Format, PageDto, PageKind};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Versioned namespace for file-mode runtime block locators. These UUIDs are
/// store/UI keys only: persisted `id::` remains the external `((id))` identity.
const FILE_BLOCK_RUNTIME_NAMESPACE_V1: Uuid =
    Uuid::from_u128(0x1e0c_5a13_9b42_5da4_a73c_0be5_8f6a_2320);

fn normalized_runtime_owner(owner: &str) -> String {
    let owner = owner.replace('\\', "/");
    let mut parts = Vec::new();
    for part in owner.split('/') {
        match part {
            "" | "." => {}
            ".." => panic!("runtime identity owner must be graph-relative"),
            _ => parts.push(part),
        }
    }
    assert!(
        !parts.is_empty(),
        "runtime identity owner must not be empty"
    );
    parts.join("/")
}

fn deterministic_runtime_uuid(namespace: Uuid, name: &[u8]) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update((name.len() as u64).to_be_bytes());
    hasher.update(name);
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 variant + version 8 (application-defined deterministic UUID).
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn runtime_owner_namespace(domain: &str, owner: &str) -> Uuid {
    let owner = normalized_runtime_owner(owner);
    let mut name = Vec::with_capacity(domain.len() + owner.len() + 16);
    name.extend_from_slice(&(domain.len() as u64).to_be_bytes());
    name.extend_from_slice(domain.as_bytes());
    name.extend_from_slice(&(owner.len() as u64).to_be_bytes());
    name.extend_from_slice(owner.as_bytes());
    deterministic_runtime_uuid(FILE_BLOCK_RUNTIME_NAMESPACE_V1, &name)
}

fn assign_runtime_ids_rec(blocks: &mut [DocBlock], parent: Uuid) {
    for (sibling_index, block) in blocks.iter_mut().enumerate() {
        // Hierarchical derivation is equivalent to hashing the full sibling-index
        // path, while doing constant work per node (O(blocks)).
        let structural = deterministic_runtime_uuid(parent, &(sibling_index as u64).to_be_bytes());
        if block.uuid.is_empty() {
            block.uuid = structural.to_string();
        }
        assign_runtime_ids_rec(&mut block.children, structural);
    }
}

/// Seed missing runtime keys for a graph-backed document from its normalized,
/// graph-relative physical owner. Existing live keys survive ordinary saves.
pub fn assign_doc_runtime_ids(roots: &mut [DocBlock], owner_rel_path: &str) {
    let owner = runtime_owner_namespace("file-block-runtime-v1", owner_rel_path);
    assign_runtime_ids_rec(roots, owner);
}

fn assign_virtual_doc_runtime_ids(roots: &mut [DocBlock], domain: &str, owner: &str) {
    let owner = runtime_owner_namespace(domain, owner);
    assign_runtime_ids_rec(roots, owner);
}

fn block_runtime_id(b: &DocBlock) -> String {
    assert!(
        !b.uuid.is_empty(),
        "DocBlock must have an explicit runtime owner before DTO projection"
    );
    b.uuid.clone()
}

/// Convert a parsed (cached) block to a DTO, carrying its stable uuid as the id.
pub fn block_to_dto(b: &DocBlock) -> BlockDto {
    BlockDto {
        id: block_runtime_id(b),
        raw: b.raw.clone(),
        collapsed: b.collapsed(),
        children: b.children.iter().map(block_to_dto).collect(),
        breadcrumb: Vec::new(),
        page_property: false,
        // All header facets off the one lsdoc projection (marker/priority/heading/
        // properties/scheduled/deadline) — priority is header-position only, matching
        // the chip, so a loaded block never shows a priority the edit path wouldn't.
        marker: b.marker().map(str::to_string),
        priority: b.priority().map(str::to_string),
        heading_level: b.heading_level(),
        scheduled: b.scheduled().map(str::to_string),
        deadline: b.deadline().map(str::to_string),
        tags: b.tags(),
        properties: b.properties(),
    }
}

/// Convert one block to the result-row wire shape. Result membership is about
/// block identity, raw text, and facets; descendants belong to the source page
/// and are hydrated once per page by live consumers. Keeping this constructor
/// separate makes it difficult to accidentally reintroduce overlapping subtree
/// amplification in queries, references, search, or batched resolution.
pub fn block_to_shallow_dto(b: &DocBlock) -> BlockDto {
    BlockDto {
        id: block_runtime_id(b),
        raw: b.raw.clone(),
        collapsed: b.collapsed(),
        children: Vec::new(),
        breadcrumb: Vec::new(),
        page_property: false,
        marker: b.marker().map(str::to_string),
        priority: b.priority().map(str::to_string),
        heading_level: b.heading_level(),
        scheduled: b.scheduled().map(str::to_string),
        deadline: b.deadline().map(str::to_string),
        tags: b.tags(),
        properties: b.properties(),
    }
}

/// Build a Markdown page DTO from raw Logseq Markdown without touching disk.
/// Used by the bundled in-app Guide so it reuses the same document parser and
/// DTO projection as normal graph pages.
pub fn markdown_page_dto(name: &str, title: &str, markdown: &str) -> PageDto {
    let mut doc = doc::parse(markdown);
    assign_virtual_doc_runtime_ids(&mut doc.roots, "bundled-markdown-v1", name);
    PageDto {
        name: name.to_string(),
        kind: PageKind::Page,
        title: title.to_string(),
        pre_block: doc.pre_block.clone(),
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    }
}
