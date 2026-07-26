//! Versioned policy for ordinary Logseq-compatible graph text discovery.
//!
//! This module answers only whether an existing graph-relative path belongs to
//! the readable/indexable text scope. It deliberately grants no creation,
//! projection, enrollment, rename, or deletion authority.

use crate::oplog::{
    managed_component_is_portable, CanonicalGraphResourceId, PortablePathKey,
    PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION, PORTABLE_PATH_KEY_VERSION,
    PORTABLE_PATH_NORMALIZATION_UNICODE_VERSION,
};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

/// Version bound by later watcher, enrollment, backup, and restore packets.
pub const GRAPH_TEXT_SCOPE_VERSION: u32 = 1;
pub const GRAPH_TEXT_SCOPE_BINDING_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_HIDDEN_EDN_BYTES: usize = 64 * 1024;
pub(crate) const MAX_HIDDEN_EDN_ENTRIES: usize = 1024;
pub(crate) const MAX_HIDDEN_EDN_DEPTH: usize = 64;
pub(crate) const MAX_HIDDEN_EDN_FORMS: usize = 4096;

const BINDING_SCHEMA_OFFSET: usize = 0;
const POLICY_VERSION_OFFSET: usize = 4;
const PORTABLE_PATH_KEY_VERSION_OFFSET: usize = 8;
const NORMALIZATION_UNICODE_VERSION_OFFSET: usize = 12;
const CASE_FOLD_UNICODE_VERSION_OFFSET: usize = 36;
const GRAPH_RESOURCE_ID_OFFSET: usize = 60;
const EFFECTIVE_POLICY_DIGEST_OFFSET: usize = 92;
const WITNESS_OFFSET: usize = 124;
const GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES: usize = 156;

#[derive(Debug, Clone)]
pub struct GraphTextScope {
    hidden_prefixes: HiddenPrefixTrie,
    hide_all: bool,
    canonical_hidden_prefixes: Vec<Vec<u8>>,
    effective_policy_digest: [u8; 32],
}

/// Opaque identity of one effective graph-text policy installed for one
/// retained graph-root resource.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct GraphTextScopeBinding {
    binding_schema_version: u32,
    policy_version: u32,
    portable_path_key_version: u32,
    normalization_unicode_version: (u64, u64, u64),
    case_fold_unicode_version: (u64, u64, u64),
    graph_resource_id: CanonicalGraphResourceId,
    effective_policy_digest: [u8; 32],
    witness: [u8; 32],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphTextScopeBindingWire {
    binding_schema_version: u32,
    policy_version: u32,
    portable_path_key_version: u32,
    normalization_unicode_version: (u64, u64, u64),
    case_fold_unicode_version: (u64, u64, u64),
    graph_resource_id: CanonicalGraphResourceId,
    effective_policy_digest: [u8; 32],
    witness: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphTextScopeBindingError {
    InvalidCanonicalLength(usize),
    UnknownBindingSchemaVersion(u32),
    UnknownPolicyVersion(u32),
    UnknownPortablePathKeyVersion(u32),
    UnknownNormalizationUnicodeVersion((u64, u64, u64)),
    UnknownCaseFoldUnicodeVersion((u64, u64, u64)),
    WitnessMismatch,
}

impl fmt::Display for GraphTextScopeBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCanonicalLength(found) => write!(
                formatter,
                "graph-text scope binding has {found} canonical bytes; expected {GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES}"
            ),
            Self::UnknownBindingSchemaVersion(found) => write!(
                formatter,
                "unknown graph-text scope binding schema {found}; expected {GRAPH_TEXT_SCOPE_BINDING_SCHEMA_VERSION}"
            ),
            Self::UnknownPolicyVersion(found) => write!(
                formatter,
                "unknown graph-text scope policy {found}; expected {GRAPH_TEXT_SCOPE_VERSION}"
            ),
            Self::UnknownPortablePathKeyVersion(found) => write!(
                formatter,
                "unknown portable path key version {found}; expected {PORTABLE_PATH_KEY_VERSION}"
            ),
            Self::UnknownNormalizationUnicodeVersion(found) => write!(
                formatter,
                "unknown portable path normalization Unicode version {found:?}; expected {:?}",
                widened_normalization_unicode_version()
            ),
            Self::UnknownCaseFoldUnicodeVersion(found) => write!(
                formatter,
                "unknown portable path case-fold Unicode version {found:?}; expected {PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION:?}"
            ),
            Self::WitnessMismatch => {
                formatter.write_str("graph-text scope binding witness mismatch")
            }
        }
    }
}

impl std::error::Error for GraphTextScopeBindingError {}

impl<'de> Deserialize<'de> for GraphTextScopeBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = GraphTextScopeBindingWire::deserialize(deserializer)?;
        Self::from_fields(
            wire.binding_schema_version,
            wire.policy_version,
            wire.portable_path_key_version,
            wire.normalization_unicode_version,
            wire.case_fold_unicode_version,
            wire.graph_resource_id,
            wire.effective_policy_digest,
            wire.witness,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl GraphTextScopeBinding {
    pub const CANONICAL_BYTES_LENGTH: usize = GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES;

    pub const fn binding_schema_version(&self) -> u32 {
        self.binding_schema_version
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub const fn portable_path_key_version(&self) -> u32 {
        self.portable_path_key_version
    }

    pub const fn normalization_unicode_version(&self) -> (u64, u64, u64) {
        self.normalization_unicode_version
    }

    pub const fn case_fold_unicode_version(&self) -> (u64, u64, u64) {
        self.case_fold_unicode_version
    }

    pub const fn graph_resource_id(&self) -> CanonicalGraphResourceId {
        self.graph_resource_id
    }

    pub const fn effective_policy_digest(&self) -> &[u8; 32] {
        &self.effective_policy_digest
    }

    pub const fn witness(&self) -> &[u8; 32] {
        &self.witness
    }

    pub fn canonical_bytes(&self) -> [u8; GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES] {
        let mut bytes = [0; GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES];
        write_u32(
            &mut bytes,
            BINDING_SCHEMA_OFFSET,
            self.binding_schema_version,
        );
        write_u32(&mut bytes, POLICY_VERSION_OFFSET, self.policy_version);
        write_u32(
            &mut bytes,
            PORTABLE_PATH_KEY_VERSION_OFFSET,
            self.portable_path_key_version,
        );
        write_unicode_version(
            &mut bytes,
            NORMALIZATION_UNICODE_VERSION_OFFSET,
            self.normalization_unicode_version,
        );
        write_unicode_version(
            &mut bytes,
            CASE_FOLD_UNICODE_VERSION_OFFSET,
            self.case_fold_unicode_version,
        );
        bytes[GRAPH_RESOURCE_ID_OFFSET..EFFECTIVE_POLICY_DIGEST_OFFSET]
            .copy_from_slice(self.graph_resource_id.as_bytes());
        bytes[EFFECTIVE_POLICY_DIGEST_OFFSET..WITNESS_OFFSET]
            .copy_from_slice(&self.effective_policy_digest);
        bytes[WITNESS_OFFSET..].copy_from_slice(&self.witness);
        bytes
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, GraphTextScopeBindingError> {
        if bytes.len() != GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES {
            return Err(GraphTextScopeBindingError::InvalidCanonicalLength(
                bytes.len(),
            ));
        }
        let resource_id = canonical_graph_resource_id_from_bytes(
            bytes[GRAPH_RESOURCE_ID_OFFSET..EFFECTIVE_POLICY_DIGEST_OFFSET]
                .try_into()
                .expect("binding length was checked"),
        );
        Self::from_fields(
            read_u32(bytes, BINDING_SCHEMA_OFFSET),
            read_u32(bytes, POLICY_VERSION_OFFSET),
            read_u32(bytes, PORTABLE_PATH_KEY_VERSION_OFFSET),
            read_unicode_version(bytes, NORMALIZATION_UNICODE_VERSION_OFFSET),
            read_unicode_version(bytes, CASE_FOLD_UNICODE_VERSION_OFFSET),
            resource_id,
            bytes[EFFECTIVE_POLICY_DIGEST_OFFSET..WITNESS_OFFSET]
                .try_into()
                .expect("binding length was checked"),
            bytes[WITNESS_OFFSET..]
                .try_into()
                .expect("binding length was checked"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        binding_schema_version: u32,
        policy_version: u32,
        portable_path_key_version: u32,
        normalization_unicode_version: (u64, u64, u64),
        case_fold_unicode_version: (u64, u64, u64),
        graph_resource_id: CanonicalGraphResourceId,
        effective_policy_digest: [u8; 32],
        witness: [u8; 32],
    ) -> Result<Self, GraphTextScopeBindingError> {
        if binding_schema_version != GRAPH_TEXT_SCOPE_BINDING_SCHEMA_VERSION {
            return Err(GraphTextScopeBindingError::UnknownBindingSchemaVersion(
                binding_schema_version,
            ));
        }
        if policy_version != GRAPH_TEXT_SCOPE_VERSION {
            return Err(GraphTextScopeBindingError::UnknownPolicyVersion(
                policy_version,
            ));
        }
        if portable_path_key_version != PORTABLE_PATH_KEY_VERSION {
            return Err(GraphTextScopeBindingError::UnknownPortablePathKeyVersion(
                portable_path_key_version,
            ));
        }
        if normalization_unicode_version != widened_normalization_unicode_version() {
            return Err(
                GraphTextScopeBindingError::UnknownNormalizationUnicodeVersion(
                    normalization_unicode_version,
                ),
            );
        }
        if case_fold_unicode_version != PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION {
            return Err(GraphTextScopeBindingError::UnknownCaseFoldUnicodeVersion(
                case_fold_unicode_version,
            ));
        }
        let expected_witness = binding_witness(
            binding_schema_version,
            policy_version,
            portable_path_key_version,
            normalization_unicode_version,
            case_fold_unicode_version,
            graph_resource_id,
            effective_policy_digest,
        );
        if witness != expected_witness {
            return Err(GraphTextScopeBindingError::WitnessMismatch);
        }
        Ok(Self {
            binding_schema_version,
            policy_version,
            portable_path_key_version,
            normalization_unicode_version,
            case_fold_unicode_version,
            graph_resource_id,
            effective_policy_digest,
            witness,
        })
    }
}

impl GraphTextScope {
    pub fn new(configured_hidden: &[String], hidden_parse_failed_closed: bool) -> Self {
        let mut hidden_prefixes = HiddenPrefixTrie::default();
        let mut canonical_hidden_prefixes = Vec::new();
        let mut hide_all = hidden_parse_failed_closed;
        let mut retained_bytes = 0usize;
        if configured_hidden.len() > MAX_HIDDEN_EDN_ENTRIES {
            hide_all = true;
        }
        for configured in configured_hidden {
            if configured.is_empty() {
                // OG turns an empty configured pattern into "/" and therefore
                // hides every graph-relative path.
                hide_all = true;
                continue;
            }
            if configured.starts_with('/') {
                // Hidden prefixes are graph-relative. A leading separator is an
                // invalid alias, including the otherwise-ambiguous spelling "/".
                continue;
            }
            // OG accepts a directory spelling with one optional trailing slash.
            // Normalize exactly one: a repeated separator still contains an
            // empty component and is rejected by `lexical_components`.
            let configured = configured.strip_suffix('/').unwrap_or(configured);
            let Some(components) = lexical_components(configured) else {
                // OG compares normalized forward-slash graph paths literally.
                // An entry outside the admitted lexical alphabet cannot match an
                // eligible document and is therefore source-compatibly inert.
                continue;
            };
            debug_assert!(!components.is_empty());
            retained_bytes = match retained_bytes.checked_add(configured.len()) {
                Some(bytes) if bytes <= MAX_HIDDEN_EDN_BYTES => bytes,
                _ => {
                    hide_all = true;
                    break;
                }
            };
            hidden_prefixes.insert(configured.as_bytes());
            canonical_hidden_prefixes.push(configured.as_bytes().to_vec());
        }
        if hide_all {
            canonical_hidden_prefixes.clear();
        } else {
            canonical_hidden_prefixes.sort_unstable();
            canonical_hidden_prefixes.dedup();
        }
        let effective_policy_digest = effective_policy_digest(hide_all, &canonical_hidden_prefixes);
        Self {
            hidden_prefixes,
            hide_all,
            canonical_hidden_prefixes,
            effective_policy_digest,
        }
    }

    pub const fn version(&self) -> u32 {
        GRAPH_TEXT_SCOPE_VERSION
    }

    pub(crate) fn bind_graph_resource(
        &self,
        graph_resource_id: CanonicalGraphResourceId,
    ) -> GraphTextScopeBinding {
        debug_assert!(self.hide_all || self.canonical_hidden_prefixes.is_sorted());
        let binding_schema_version = GRAPH_TEXT_SCOPE_BINDING_SCHEMA_VERSION;
        let policy_version = GRAPH_TEXT_SCOPE_VERSION;
        let portable_path_key_version = PORTABLE_PATH_KEY_VERSION;
        let normalization_unicode_version = widened_normalization_unicode_version();
        let case_fold_unicode_version = PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION;
        let witness = binding_witness(
            binding_schema_version,
            policy_version,
            portable_path_key_version,
            normalization_unicode_version,
            case_fold_unicode_version,
            graph_resource_id,
            self.effective_policy_digest,
        );
        GraphTextScopeBinding {
            binding_schema_version,
            policy_version,
            portable_path_key_version,
            normalization_unicode_version,
            case_fold_unicode_version,
            graph_resource_id,
            effective_policy_digest: self.effective_policy_digest,
            witness,
        }
    }

    /// True when a directory may contain eligible descendants.
    pub fn should_descend(&self, relative: &str) -> bool {
        let Some(components) = lexical_components(relative) else {
            return false;
        };
        if components.is_empty() || self.hide_all {
            return components.is_empty() && !self.hide_all;
        }
        !fixed_excluded(&components)
            && !components
                .iter()
                .any(|component| component.starts_with('.'))
            && !self.hidden(relative)
    }

    /// True for one existing regular text document in graph-relative spelling.
    pub fn is_eligible(&self, relative: &str) -> bool {
        let Some(components) = lexical_components(relative) else {
            return false;
        };
        let Some(filename) = components.last() else {
            return false;
        };
        if self.hide_all
            || fixed_excluded(&components)
            || components
                .iter()
                .any(|component| component.starts_with('.'))
            || self.hidden(relative)
            || provider_conflict_copy(filename)
        {
            return false;
        }
        filename
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("md")
                    || extension.eq_ignore_ascii_case("markdown")
                    || extension.eq_ignore_ascii_case("org")
            })
    }

    /// Portable case/NFC key for collision detection.
    pub fn portable_path_key(&self, relative: &str) -> Option<String> {
        self.is_eligible(relative).then(|| {
            PortablePathKey::from_graph_text_path(relative)
                .as_str()
                .to_owned()
        })
    }

    fn hidden(&self, relative: &str) -> bool {
        self.hidden_prefixes.matches(relative.as_bytes())
    }
}

const fn widened_normalization_unicode_version() -> (u64, u64, u64) {
    let (major, minor, patch) = PORTABLE_PATH_NORMALIZATION_UNICODE_VERSION;
    (major as u64, minor as u64, patch as u64)
}

fn hash_unicode_version(hasher: &mut Sha256, version: (u64, u64, u64)) {
    hasher.update(version.0.to_be_bytes());
    hasher.update(version.1.to_be_bytes());
    hasher.update(version.2.to_be_bytes());
}

fn effective_policy_digest(hide_all: bool, canonical_prefixes: &[Vec<u8>]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/graph-text-scope/effective-policy/v1\0");
    hasher.update(GRAPH_TEXT_SCOPE_VERSION.to_be_bytes());
    hasher.update(PORTABLE_PATH_KEY_VERSION.to_be_bytes());
    hash_unicode_version(&mut hasher, widened_normalization_unicode_version());
    hash_unicode_version(&mut hasher, PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION);
    hasher.update([u8::from(hide_all)]);
    hasher.update((canonical_prefixes.len() as u64).to_be_bytes());
    for prefix in canonical_prefixes {
        hasher.update((prefix.len() as u64).to_be_bytes());
        hasher.update(prefix);
    }
    hasher.finalize().into()
}

#[allow(clippy::too_many_arguments)]
fn binding_witness(
    binding_schema_version: u32,
    policy_version: u32,
    portable_path_key_version: u32,
    normalization_unicode_version: (u64, u64, u64),
    case_fold_unicode_version: (u64, u64, u64),
    graph_resource_id: CanonicalGraphResourceId,
    effective_policy_digest: [u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/graph-text-scope-binding/v1\0");
    hasher.update(binding_schema_version.to_be_bytes());
    hasher.update(policy_version.to_be_bytes());
    hasher.update(portable_path_key_version.to_be_bytes());
    hash_unicode_version(&mut hasher, normalization_unicode_version);
    hash_unicode_version(&mut hasher, case_fold_unicode_version);
    hasher.update(graph_resource_id.as_bytes());
    hasher.update(effective_policy_digest);
    hasher.finalize().into()
}

fn write_u32(
    bytes: &mut [u8; GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES],
    offset: usize,
    value: u32,
) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_unicode_version(
    bytes: &mut [u8; GRAPH_TEXT_SCOPE_BINDING_CANONICAL_BYTES],
    offset: usize,
    version: (u64, u64, u64),
) {
    bytes[offset..offset + 8].copy_from_slice(&version.0.to_be_bytes());
    bytes[offset + 8..offset + 16].copy_from_slice(&version.1.to_be_bytes());
    bytes[offset + 16..offset + 24].copy_from_slice(&version.2.to_be_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("binding length was checked"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("binding length was checked"),
    )
}

fn read_unicode_version(bytes: &[u8], offset: usize) -> (u64, u64, u64) {
    (
        read_u64(bytes, offset),
        read_u64(bytes, offset + 8),
        read_u64(bytes, offset + 16),
    )
}

fn canonical_graph_resource_id_from_bytes(bytes: &[u8; 32]) -> CanonicalGraphResourceId {
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    CanonicalGraphResourceId::from_str(&encoded)
        .expect("all fixed-size byte strings have a canonical digest encoding")
}

#[derive(Debug, Clone, Default)]
struct HiddenPrefixTrie {
    nodes: Vec<HiddenPrefixNode>,
}

#[derive(Debug, Clone, Default)]
struct HiddenPrefixNode {
    terminal: bool,
    children: HashMap<u8, usize>,
}

impl HiddenPrefixTrie {
    fn insert(&mut self, prefix: &[u8]) {
        if self.nodes.is_empty() {
            self.nodes.push(HiddenPrefixNode::default());
        }
        let mut node = 0usize;
        for byte in prefix {
            let next = match self.nodes[node].children.get(byte).copied() {
                Some(next) => next,
                None => {
                    let next = self.nodes.len();
                    self.nodes.push(HiddenPrefixNode::default());
                    self.nodes[node].children.insert(*byte, next);
                    next
                }
            };
            node = next;
        }
        self.nodes[node].terminal = true;
    }

    fn matches(&self, relative: &[u8]) -> bool {
        let Some(root) = self.nodes.first() else {
            return false;
        };
        if root.terminal {
            return true;
        }
        let mut node = 0usize;
        for byte in relative {
            let Some(next) = self.nodes[node].children.get(byte).copied() else {
                return false;
            };
            node = next;
            if self.nodes[node].terminal {
                return true;
            }
        }
        false
    }
}

fn lexical_components(relative: &str) -> Option<Vec<&str>> {
    if relative != relative.trim()
        || relative.is_empty()
        || relative.starts_with('/')
        || relative.contains('\\')
        || relative.contains('\0')
    {
        return None;
    }
    let components = relative.split('/').collect::<Vec<_>>();
    components
        .iter()
        .all(|component| managed_component_is_portable(component))
        .then_some(components)
}

fn starts_with(components: &[&str], prefix: &[&str]) -> bool {
    components.len() >= prefix.len()
        && components
            .iter()
            .zip(prefix)
            .all(|(component, prefix)| component.eq_ignore_ascii_case(prefix))
}

fn fixed_excluded(components: &[&str]) -> bool {
    if components
        .iter()
        .any(|component| component.eq_ignore_ascii_case("node_modules"))
        || starts_with(components, &["assets"])
        || starts_with(components, &["publish"])
        || starts_with(components, &[".tine-sync"])
        || starts_with(components, &["logseq", ".recycle"])
        || starts_with(components, &["logseq", "bak"])
        || starts_with(components, &["logseq", "version-files"])
        || starts_with(components, &["logseq", ".tine-trash"])
    {
        return true;
    }
    components.len() == 2
        && components[0].eq_ignore_ascii_case("logseq")
        && matches!(
            components[1].to_ascii_lowercase().as_str(),
            "graphs-txid.edn" | "pages-metadata.edn"
        )
}

fn provider_conflict_copy(filename: &str) -> bool {
    let stem = filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename);
    crate::model::is_sync_conflict(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_wide_policy_matches_fixed_and_hidden_prefix_contract() {
        let scope = GraphTextScope::new(
            &[
                "archive/private".into(),
                "scratch".into(),
                "../assets/archived".into(),
                "bad\\windows".into(),
            ],
            false,
        );
        for accepted in [
            "root.md",
            "UPPER.MD",
            "mixed.Markdown",
            "outline.ORG",
            "deep/a/page.markdown",
            "deep/b/page.org",
            "logseq/allowed.md",
            "outside/page.md",
            "safe/bad/page.md",
        ] {
            assert!(scope.is_eligible(accepted), "{accepted}");
        }
        for rejected in [
            ".hidden.md",
            "deep/.hidden/page.md",
            "node_modules/package/readme.md",
            "deep/node_modules/package/readme.md",
            "logseq/.recycle/page.md",
            "logseq/bak/page.md",
            "logseq/version-files/page.md",
            "logseq/graphs-txid.edn",
            "logseq/pages-metadata.edn",
            "assets/page.md",
            "publish/page.org",
            ".tine-sync/page.md",
            "logseq/.tine-trash/pages/page.md",
            "archive/private/page.md",
            "scratch/nested/page.org",
            "page.sync-conflict-20260725-120000-ABCDEF.md",
            "page (conflicted copy 2026-07-25).md",
        ] {
            assert!(!scope.is_eligible(rejected), "{rejected}");
        }
    }

    #[test]
    fn one_trailing_hidden_separator_matches_the_unseparated_literal_prefix() {
        let plain = GraphTextScope::new(&["archive".into()], false);
        let trailing = GraphTextScope::new(&["archive/".into()], false);
        for directory in ["archive", "archive/nested", "archive-old"] {
            assert_eq!(
                trailing.should_descend(directory),
                plain.should_descend(directory),
                "{directory}"
            );
        }
        for path in [
            "archive/page.md",
            "archive/nested/page.org",
            "archive-old/page.markdown",
            "elsewhere/archive/page.md",
        ] {
            assert_eq!(
                trailing.is_eligible(path),
                plain.is_eligible(path),
                "{path}"
            );
        }
        assert!(!trailing.should_descend("archive"));
        assert!(!trailing.is_eligible("archive/page.md"));
        assert!(!trailing.is_eligible("archive-old/page.md"));
        assert!(trailing.is_eligible("elsewhere/archive/page.md"));

        for invalid in [
            "/",
            "/archive",
            "../archive",
            "archive/../private",
            "archive/./private",
            "archive//",
            "archive///",
            "archive//nested",
            "archive\\nested",
            " archive",
            "archive ",
        ] {
            let scope = GraphTextScope::new(&[invalid.into()], false);
            assert!(
                scope.is_eligible("archive/page.md"),
                "invalid hidden alias must be inert: {invalid}"
            );
        }
    }

    #[test]
    fn empty_hidden_pattern_matches_og_hide_all_behavior() {
        let scope = GraphTextScope::new(&["".into()], false);
        assert!(!scope.is_eligible("pages/page.md"));
    }

    #[test]
    fn portable_key_folds_case_and_nfc() {
        let scope = GraphTextScope::new(&[], false);
        assert!(scope.portable_path_key("Root.markdown").is_some());
        assert_eq!(
            scope.portable_path_key("Archive/Café.md"),
            scope.portable_path_key("archive/Cafe\u{301}.md")
        );
        assert_eq!(
            scope.portable_path_key("external/Straße.md"),
            scope.portable_path_key("external/STRASSE.md")
        );
        assert_eq!(
            scope.portable_path_key("external/Σ.md"),
            scope.portable_path_key("external/ς.md")
        );
    }

    #[test]
    fn malformed_or_over_limit_hidden_policy_fails_closed() {
        let malformed = GraphTextScope::new(&[], true);
        assert!(!malformed.is_eligible("pages/page.md"));

        let over_limit = vec!["x".to_owned(); MAX_HIDDEN_EDN_ENTRIES + 1];
        let bounded = GraphTextScope::new(&over_limit, false);
        assert!(!bounded.is_eligible("pages/page.md"));
    }

    #[test]
    fn binding_canonicalizes_effective_hidden_policy_and_graph_resource() {
        let resource =
            CanonicalGraphResourceId::from_capability_identity(b"test", b"same-resource");
        let reordered = GraphTextScope::new(
            &[
                "scratch".into(),
                "/inert".into(),
                "archive/".into(),
                "archive".into(),
                "bad\\windows".into(),
            ],
            false,
        )
        .bind_graph_resource(resource);
        let canonical = GraphTextScope::new(&["archive".into(), "scratch".into()], false)
            .bind_graph_resource(resource);
        assert_eq!(reordered, canonical);

        let hide_all_from_empty = GraphTextScope::new(&["".into(), "archive".into()], false)
            .bind_graph_resource(resource);
        let hide_all_from_parse_failure =
            GraphTextScope::new(&["different".into()], true).bind_graph_resource(resource);
        assert_eq!(hide_all_from_empty, hide_all_from_parse_failure);

        let changed =
            GraphTextScope::new(&["elsewhere".into()], false).bind_graph_resource(resource);
        assert_ne!(canonical, changed);
        let copied_resource = CanonicalGraphResourceId::from_capability_identity(
            b"test",
            b"copied-directory-resource",
        );
        assert_ne!(
            canonical,
            GraphTextScope::new(&["archive".into(), "scratch".into()], false)
                .bind_graph_resource(copied_resource)
        );
    }

    #[test]
    fn binding_canonical_bytes_and_serde_round_trip_but_reject_forgery() {
        let resource =
            CanonicalGraphResourceId::from_capability_identity(b"test", b"round-trip-resource");
        let binding =
            GraphTextScope::new(&["archive/".into()], false).bind_graph_resource(resource);
        let bytes = binding.canonical_bytes();
        assert_eq!(bytes.len(), GraphTextScopeBinding::CANONICAL_BYTES_LENGTH);
        assert_eq!(
            GraphTextScopeBinding::from_canonical_bytes(&bytes).unwrap(),
            binding
        );
        let json = serde_json::to_vec(&binding).unwrap();
        assert_eq!(
            serde_json::from_slice::<GraphTextScopeBinding>(&json).unwrap(),
            binding
        );

        for index in [
            BINDING_SCHEMA_OFFSET,
            POLICY_VERSION_OFFSET,
            PORTABLE_PATH_KEY_VERSION_OFFSET,
            NORMALIZATION_UNICODE_VERSION_OFFSET,
            CASE_FOLD_UNICODE_VERSION_OFFSET,
            EFFECTIVE_POLICY_DIGEST_OFFSET,
            WITNESS_OFFSET,
        ] {
            let mut forged = bytes;
            forged[index] ^= 1;
            assert!(GraphTextScopeBinding::from_canonical_bytes(&forged).is_err());
        }
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert!(GraphTextScopeBinding::from_canonical_bytes(&trailing).is_err());
    }
}
