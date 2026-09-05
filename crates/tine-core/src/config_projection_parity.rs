//! `Config` is what `logseq/config.edn` says; `GraphMeta` is what the frontend
//! is told. Nothing checked that the second still covers the first.
//!
//! Three declarations have to agree for a setting to reach the UI: the `Config`
//! field parsed out of `config.edn`, the `GraphMeta` field the backend sends,
//! and the `GraphMeta` field the TypeScript side reads. Adding a setting means
//! editing all three, and forgetting the second or the third is silent — the
//! setting simply never appears, and the only symptom is a user asking why
//! their config.edn key does nothing.
//!
//! This module reads all three declarations out of the sources at test time and
//! makes a missing projection a failing diff. It pins *field coverage*: which
//! settings cross the boundary. Types, serialization shape and defaults are not
//! its business.

#[cfg(test)]
use std::collections::BTreeSet;

#[cfg(test)]
const CONFIG_RS: &str = include_str!("config.rs");

#[cfg(test)]
const MODEL_RS: &str = include_str!("model.rs");

/// The TypeScript side of the wire. The path leaves the crate deliberately —
/// the whole point is that the frontend declaration is the third copy, and a
/// guard that cannot see it is not a guard. `#[cfg(test)]` keeps it out of
/// every non-test build.
#[cfg(test)]
const TYPES_TS: &str = include_str!("../../../src/types.ts");

/// `Config` fields that are deliberately NOT projected into `GraphMeta`.
///
/// Each entry says why the frontend does not need it. "Nobody asked for it" is
/// not a reason — if the frontend would use it, project it instead.
#[cfg(test)]
const CONFIG_FIELDS_NOT_PROJECTED: &[(&str, &str)] = &[
    (
        "hidden",
        "`:hidden` is enforced in the backend: hidden paths never enter the page \
         list or search results, so the frontend has nothing to filter",
    ),
    (
        "hidden_parse_failed_closed",
        "an internal fail-closed flag recording that `:hidden` could not be \
         parsed; the backend hides everything in that case and the frontend \
         cannot act on the distinction",
    ),
    (
        "all_pages_public",
        "consumed by the publish/export path in the backend \
         (`crate::publish`), which decides page visibility before any HTML \
         reaches the frontend",
    ),
    (
        "property_pages_enabled",
        "governs whether the backend materializes property pages at all; the \
         frontend sees the resulting page set, not the switch",
    ),
    (
        "property_pages_excludelist",
        "the exclusion list for the same backend-side decision",
    ),
    (
        "file_name_format",
        "`:file/name-format` selects the backend's namespace filename encoding. \
         Filenames are the backend's boundary with disk; the frontend addresses \
         pages by name",
    ),
    (
        "logbook",
        "flattened rather than dropped: `LogbookSettings` is projected as the \
         three scalar `logbook_*` fields below, which the parity test checks",
    ),
    (
        "separated_by_commas",
        "`:property/separated-by-commas` is a parser input: it reaches the query \
         engine as part of `ParseConfig` and decides how a property VALUE splits \
         into atoms. The frontend reads the already-split atoms, never the rule",
    ),
    (
        "ignored_page_references_keywords",
        "`:ignored-page-references-keywords` is the same kind of parser input — \
         it suppresses reference extraction for the named property keys inside \
         `ParseConfig`, before anything the frontend can see exists",
    ),
];

/// The `logbook` sub-settings, flattened onto `GraphMeta`. Named here so the
/// `logbook` allowlist entry above is a claim the test actually verifies.
#[cfg(test)]
const LOGBOOK_PROJECTED_AS: &[&str] = &[
    "logbook_with_second_support",
    "logbook_enabled_in_timestamped_blocks",
    "logbook_enabled_in_all_blocks",
];

/// `GraphMeta` fields with no `Config` field behind them.
#[cfg(test)]
const GRAPH_META_FIELDS_WITHOUT_CONFIG: &[(&str, &str)] = &[(
    "root",
    "the graph's filesystem root — where the config came FROM, not a \
         setting inside it",
)];

/// Field names of a `pub struct NAME { … }` in a Rust source, in declaration
/// order. Only `pub` fields count: a private field is not part of the surface
/// this guard is about.
#[cfg(test)]
fn rust_struct_pub_fields(source: &str, name: &str) -> Vec<String> {
    let header = format!("pub struct {name} {{");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("`{header}` not found -- the struct was renamed or moved"));
    let body_start = start + header.len();
    let end = source[body_start..]
        .find("\n}")
        .unwrap_or_else(|| panic!("unterminated `{name}` struct body"))
        + body_start;

    let mut fields = Vec::new();
    for line in source[body_start..end].lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub ") else {
            continue;
        };
        let Some((field, _)) = rest.split_once(':') else {
            continue;
        };
        let field = field.trim();
        if !field.is_empty() && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            fields.push(field.to_owned());
        }
    }
    assert!(!fields.is_empty(), "scanned no pub fields out of `{name}`");
    fields
}

/// Property names of a TypeScript `export interface NAME { … }`, in declaration
/// order. Optional properties (`x?: T`) count — optionality is a serialization
/// question, not a coverage one.
#[cfg(test)]
fn ts_interface_fields(source: &str, name: &str) -> Vec<String> {
    let header = format!("export interface {name} {{");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("`{header}` not found in src/types.ts"));
    let body_start = start + header.len();
    let end = source[body_start..]
        .find("\n}")
        .unwrap_or_else(|| panic!("unterminated `{name}` interface body"))
        + body_start;

    let mut fields = Vec::new();
    for line in source[body_start..end].lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with('*')
            || line.starts_with("/*")
        {
            continue;
        }
        let Some((field, _)) = line.split_once(':') else {
            continue;
        };
        let field = field.trim().trim_end_matches('?');
        if !field.is_empty() && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            fields.push(field.to_owned());
        }
    }
    assert!(
        !fields.is_empty(),
        "scanned no fields out of the TS `{name}` interface"
    );
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_fields() -> Vec<String> {
        rust_struct_pub_fields(CONFIG_RS, "Config")
    }

    fn graph_meta_rust() -> Vec<String> {
        rust_struct_pub_fields(MODEL_RS, "GraphMeta")
    }

    fn graph_meta_ts() -> Vec<String> {
        ts_interface_fields(TYPES_TS, "GraphMeta")
    }

    /// Every setting parsed out of `config.edn` either reaches the frontend or
    /// says, here, why it does not.
    #[test]
    fn every_config_field_is_projected_or_explained() {
        let rust: BTreeSet<String> = graph_meta_rust().into_iter().collect();
        let ts: BTreeSet<String> = graph_meta_ts().into_iter().collect();
        let excused: BTreeSet<&str> = CONFIG_FIELDS_NOT_PROJECTED
            .iter()
            .map(|(field, _)| *field)
            .collect();

        let mut missing_in_rust = Vec::new();
        let mut missing_in_ts = Vec::new();
        for field in config_fields() {
            if excused.contains(field.as_str()) {
                continue;
            }
            if !rust.contains(&field) {
                missing_in_rust.push(field.clone());
            }
            if !ts.contains(&field) {
                missing_in_ts.push(field);
            }
        }
        assert!(
            missing_in_rust.is_empty(),
            "Config fields with no GraphMeta field in crates/tine-core/src/model.rs -- the \
             setting is parsed and then never sent (add the field, or excuse it in \
             CONFIG_FIELDS_NOT_PROJECTED): {missing_in_rust:?}"
        );
        assert!(
            missing_in_ts.is_empty(),
            "Config fields with no GraphMeta property in src/types.ts -- the backend sends the \
             setting and the frontend cannot see it: {missing_in_ts:?}"
        );
    }

    /// The two `GraphMeta` declarations are one wire format written twice.
    #[test]
    fn the_rust_and_typescript_graph_meta_declare_the_same_fields() {
        let rust: BTreeSet<String> = graph_meta_rust().into_iter().collect();
        let ts: BTreeSet<String> = graph_meta_ts().into_iter().collect();
        let only_rust: Vec<&String> = rust.difference(&ts).collect();
        let only_ts: Vec<&String> = ts.difference(&rust).collect();
        assert!(
            only_rust.is_empty() && only_ts.is_empty(),
            "GraphMeta disagrees across the wire; only in Rust (model.rs): {only_rust:?}; \
             only in TypeScript (src/types.ts): {only_ts:?}"
        );
    }

    /// An excuse has to stay true. A field that was removed from `Config`, or
    /// that has since been projected, must lose its entry.
    #[test]
    fn the_unprojected_allowlist_has_no_stale_entries() {
        let config: BTreeSet<String> = config_fields().into_iter().collect();
        let rust: BTreeSet<String> = graph_meta_rust().into_iter().collect();
        let stale: Vec<&&str> = CONFIG_FIELDS_NOT_PROJECTED
            .iter()
            .map(|(field, _)| field)
            .filter(|field| !config.contains(**field) || rust.contains(**field))
            .collect();
        assert!(
            stale.is_empty(),
            "CONFIG_FIELDS_NOT_PROJECTED entries that are no longer true (gone from Config, or \
             now projected after all): {stale:?}"
        );

        let unexplained: Vec<&(&str, &str)> = GRAPH_META_FIELDS_WITHOUT_CONFIG
            .iter()
            .filter(|(field, _)| !rust.contains(*field))
            .collect();
        assert!(
            unexplained.is_empty(),
            "GRAPH_META_FIELDS_WITHOUT_CONFIG names fields GraphMeta no longer has: {unexplained:?}"
        );

        let unaccounted: Vec<&String> = rust
            .iter()
            .filter(|field| {
                !config.contains(*field)
                    && !LOGBOOK_PROJECTED_AS.contains(&field.as_str())
                    && !GRAPH_META_FIELDS_WITHOUT_CONFIG
                        .iter()
                        .any(|(known, _)| known == field)
            })
            .collect();
        assert!(
            unaccounted.is_empty(),
            "GraphMeta fields with no Config field behind them and no entry saying where they \
             come from: {unaccounted:?}"
        );
    }

    /// `logbook` is excused from the projection check because it is flattened,
    /// not dropped. Prove the flattening is really there.
    #[test]
    fn logbook_settings_are_flattened_not_dropped() {
        let rust: BTreeSet<String> = graph_meta_rust().into_iter().collect();
        let ts: BTreeSet<String> = graph_meta_ts().into_iter().collect();
        for field in LOGBOOK_PROJECTED_AS {
            assert!(
                rust.contains(*field),
                "CONFIG_FIELDS_NOT_PROJECTED excuses `logbook` as flattened, but GraphMeta \
                 (model.rs) has no `{field}`"
            );
            assert!(
                ts.contains(*field),
                "CONFIG_FIELDS_NOT_PROJECTED excuses `logbook` as flattened, but the TS \
                 GraphMeta has no `{field}`"
            );
        }
    }
}
