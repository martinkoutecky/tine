//! Versioned policy for ordinary Logseq-compatible graph text discovery.
//!
//! This module answers only whether an existing graph-relative path belongs to
//! the readable/indexable text scope. It deliberately grants no creation,
//! projection, enrollment, rename, or deletion authority.

use crate::oplog::{managed_component_is_portable, PortablePathKey};
use std::collections::HashMap;

/// Version bound by later watcher, enrollment, backup, and restore packets.
pub const GRAPH_TEXT_SCOPE_VERSION: u32 = 1;
pub(crate) const MAX_HIDDEN_EDN_BYTES: usize = 64 * 1024;
pub(crate) const MAX_HIDDEN_EDN_ENTRIES: usize = 1024;
pub(crate) const MAX_HIDDEN_EDN_DEPTH: usize = 64;
pub(crate) const MAX_HIDDEN_EDN_FORMS: usize = 4096;

#[derive(Debug, Clone)]
pub struct GraphTextScope {
    hidden_prefixes: HiddenPrefixTrie,
    hide_all: bool,
}

impl GraphTextScope {
    pub fn new(configured_hidden: &[String], hidden_parse_failed_closed: bool) -> Self {
        let mut hidden_prefixes = HiddenPrefixTrie::default();
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
        }
        Self {
            hidden_prefixes,
            hide_all,
        }
    }

    pub const fn version(&self) -> u32 {
        GRAPH_TEXT_SCOPE_VERSION
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
}
