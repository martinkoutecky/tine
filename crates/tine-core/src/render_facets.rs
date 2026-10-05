//! Which block properties are chrome rather than content, for every renderer
//! (I-12). The live app (through the WASM bridge) and the static export ask
//! this one predicate, so a property chip hidden in the app is hidden in a
//! published page and the reverse.
//!
//! Deliberately SEPARATE concepts (do not merge): the editor's textarea hide
//! list (`BUILTIN_HIDDEN` in `src/editor/properties.ts`), the query filter
//! blacklist (`INTERNAL_PROPS` in `query.rs`), and the page-property area
//! (`PAGE_PROPS_HIDDEN` in `components/Page.tsx`).
//!
//! Dependency-free on purpose: `lsdoc-wasm` includes this file by path.

/// Built-in keys that are never shown as a rendered chip (id/collapsed, Logseq
/// internals, display-only keys), already in `normalize` form.
const RENDER_HIDDEN: &[&str] = &[
    "id",
    "collapsed",
    "hl-page",
    "hl-color",
    "hl-type",
    "ls-type",
    "background-color",
    "logseq.order-list-type",
    "heading",
    "title",
    "filters",
    "created-at",
    "updated-at",
    "last-modified-at",
    "query-table",
    "query-properties",
    "query-sort-by",
    "query-sort-desc",
    "logseq.tldraw.shape",
];

/// A property key's comparison form: trimmed, ASCII-lowercased, spaces and
/// underscores as hyphens. Byte-for-byte `doc::property_key_norm`; a test in
/// this file pins the two together because this file cannot depend on `doc`.
pub fn normalize(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

/// Whether a block property key is hidden from the rendered chips: a built-in
/// internal key, a `tine.*` view setting, a `logseq.table.*` table setting (OG
/// resolves those from the block, `shui/table/v2.cljs`), or a key the graph
/// lists in `:block-hidden-properties`. Case- and separator-insensitive.
/// O(key bytes + hidden keys), no allocation beyond the normalized key.
pub fn is_render_hidden_prop(key: &str, user_hidden: &[String]) -> bool {
    let key = normalize(key);
    key.starts_with("tine.")
        || key.starts_with("logseq.table.")
        || RENDER_HIDDEN.contains(&key.as_str())
        || user_hidden.iter().any(|hidden| normalize(hidden) == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_keys_tine_and_table_settings_are_hidden() {
        for key in ["id", "Collapsed", "Created_At", "hl color", "tine.view", "logseq.table.version", "Title"] {
            assert!(is_render_hidden_prop(key, &[]), "{key}");
        }
        for key in ["logseq.custom", "status", "tags", "alias", "public"] {
            assert!(!is_render_hidden_prop(key, &[]), "{key}");
        }
    }

    #[test]
    fn the_graphs_hidden_list_applies_after_normalization() {
        let user = vec!["My_Prop".to_owned()];
        assert!(is_render_hidden_prop("my-prop", &user));
        assert!(!is_render_hidden_prop("other", &user));
    }

    #[test]
    fn normalize_is_the_documents_property_key_norm() {
        for key in ["Id", " Hl_Color ", "A B_c", "ÉCOLE_x", "", "tine.View"] {
            assert_eq!(normalize(key), crate::doc::property_key_norm(key), "{key:?}");
        }
    }

    #[test]
    fn every_built_in_key_is_already_normalized() {
        for key in RENDER_HIDDEN {
            assert_eq!(normalize(key), *key);
        }
    }
}
