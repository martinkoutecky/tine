#[path = "support/production_source.rs"]
mod production_source;

use production_source::{compiled_source, production_source_files, relative_path, repo_root};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

const LOWERING: &str = "crates/tine-core/src/oplog/query_lowering.rs";
const DIRECT: &str = "crates/tine-core/src/direct_projection.rs";

fn sources() -> BTreeMap<String, String> {
    let root = repo_root();
    production_source_files()
        .into_iter()
        .map(|path| (relative_path(&root, &path), compiled_source(&path)))
        .collect()
}

fn function_body<'a>(source: &'a str, symbol: &str) -> &'a str {
    let needle = format!("fn {symbol}(");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("missing production symbol {symbol}"));
    let brace = source[start..].find('{').unwrap() + start;
    let mut depth = 1_usize;
    let mut end = brace + 1;
    for byte in source.as_bytes()[brace + 1..].iter() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        end += 1;
        if depth == 0 {
            return &source[start..end];
        }
    }
    panic!("unterminated production symbol {symbol}")
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DrainOwner {
    file: &'static str,
    enclosing_symbol: &'static str,
    read_family: &'static str,
    question: &'static str,
    retirement_owner: &'static str,
}

const NON_OWNED_DRAINS: &[DrainOwner] = &[
    DrainOwner {
        file: "crates/tine-core/src/oplog/operational_coordinator.rs",
        enclosing_symbol: "execute_clean_external",
        read_family: "page_inventory_after",
        question: "record_prepared_absence_batch page count",
        retirement_owner: "follow-up after W4-B4b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_block_reference_counts_ready",
        read_family: "block_reference_counts_after",
        question: "managed block reference counts",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_block_reference_counts_ready",
        read_family: "block_reference_counts_for_source_page_after",
        question: "managed source-page block reference counts",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_block_referrers_ready",
        read_family: "block_referrer_candidates_after",
        question: "managed block referrers",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_backlinks_ready",
        read_family: "page_referrer_candidates_after",
        question: "managed backlinks",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_unlinked_references_ready",
        read_family: "plain_text_candidate_pages_after",
        question: "managed unlinked references",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_templates_ready",
        read_family: "block_property_candidates_after",
        question: "managed templates",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_property_facets_ready",
        read_family: "property_facet_rows_after",
        question: "managed property facets",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_sparse_task_query_ready",
        read_family: "task_candidate_blocks_after",
        question: "managed sparse task blocks",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_fuzzy_candidate_paths_ready",
        read_family: "fuzzy_subsequence_candidate_pages_after",
        question: "managed fuzzy candidates",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_navigation_pages_ready",
        read_family: "navigation_pages_after",
        question: "managed navigation pages",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_navigation_aliases_ready",
        read_family: "navigation_aliases_after",
        question: "managed navigation aliases",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_navigation_reference_names_ready",
        read_family: "navigation_reference_names_after",
        question: "managed referenced-name inventory",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_inventory_of_kind_ready",
        read_family: "page_inventory_after",
        question: "managed inventory by kind",
        retirement_owner: "W4-C7b",
    },
    DrainOwner {
        file: "crates/tine-core/src/sync_runtime.rs",
        enclosing_symbol: "application_page_namespace_ready",
        read_family: "navigation_pages_by_name_key_namespace_after",
        question: "managed page namespace",
        retirement_owner: "W4-C7b",
    },
];

#[test]
fn hand_written_cursor_drains_are_pinned() {
    let source = sources();
    let lowering = &source[LOWERING];
    assert_eq!(
        lowering.matches("drain_after(").count(),
        10,
        "I-12: the ten SimpleQuerySqlRead consumers must delegate cursor advancement and termination to drain_after; exemplar {LOWERING}"
    );
    assert_eq!(
        lowering.matches("loop {").count(),
        1,
        "I-12: only drain_after itself may own a cursor loop in {LOWERING}"
    );

    let direct = &source[DIRECT];
    for symbol in [
        "sparse_task_query",
        "property_facets",
        "referenced_page_names",
        "fuzzy_candidate_paths",
        "page_aliases_with_owners",
        "real_page_names",
        "reference_candidate_paths",
        "block_ref_counts",
        "block_referrer_candidate_paths",
    ] {
        let body = function_body(direct, symbol);
        assert!(
            !body.contains("loop {"),
            "I-12: {DIRECT}::{symbol} retains caller-owned cursor advancement, termination, or adaptive retry; call drain_after"
        );
    }
    // 10 → 12: P0-rust Wave D's `property_owner_rows` (§6.2's Direct Files
    // registry row source) drains the page map and the property rows. Both
    // DELEGATE to `drain_after` — which is what this guard is for — so the pin
    // moves; it would be a violation only if the new consumer owned its own
    // `loop {}`, which the per-symbol assertions above still forbid.
    assert_eq!(
        direct.matches("drain_after(").count(),
        12,
        "I-12: the twelve owned Direct cursor consumers must each delegate to drain_after"
    );

    for allowed in NON_OWNED_DRAINS {
        let body = function_body(&source[allowed.file], allowed.enclosing_symbol);
        assert!(
            body.contains(allowed.read_family),
            "missing allowlisted drain {allowed:?}"
        );
        assert!(!allowed.question.is_empty() && !allowed.retirement_owner.is_empty());
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CensusRecord {
    family: String,
    file: String,
    enclosing_symbol: String,
    call_expression: String,
    class: String,
    question: String,
}

fn containing_symbol(source: &str, offset: usize) -> String {
    let before = &source[..offset];
    let function = Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([A-Za-z0-9_]+)")
        .unwrap()
        .captures_iter(before)
        .last()
        .map(|capture| capture[1].to_string())
        .expect("read-family call must be inside a named function");
    if before
        .rfind("impl SimpleQuerySqlRead for ")
        .is_some_and(|implementation| before[implementation..].rfind("fn ").is_some())
    {
        let implementation = before.rfind("impl SimpleQuerySqlRead for ").unwrap();
        let rest = &before[implementation + "impl SimpleQuerySqlRead for ".len()..];
        let owner = rest
            .split(|character: char| character.is_whitespace() || character == '{')
            .next()
            .unwrap();
        format!("{owner}::{function}")
    } else {
        function
    }
}

fn classify(file: &str, symbol: &str, family: &str) -> (&'static str, &'static str) {
    if file == LOWERING {
        return (
            "shared-lowering-adapter",
            "SimpleQueryCandidatePlan page candidates",
        );
    }
    if file == "crates/tine-core/src/oplog/sqlite_materialization.rs" {
        return (
            "facade-forwarder",
            "managed-to-physical read facade conversion",
        );
    }
    match (file, symbol, family) {
        (DIRECT, "property_facets", "property_facet_rows_after") => {
            ("other-question", "Direct property facets")
        }
        (DIRECT, "real_page_names", "navigation_pages_after_with_header_validation") => {
            ("other-question", "Direct real page ownership")
        }
        (DIRECT, "reference_candidate_paths", "page_referrer_candidates_after") => {
            ("other-question", "Direct explicit reference candidates")
        }
        (
            "crates/tine-core/src/sync_runtime.rs",
            "application_backlinks_ready",
            "page_referrer_candidates_after",
        ) => ("other-question", "managed backlinks"),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "application_templates_ready",
            "block_property_candidates_after",
        ) => ("other-question", "managed templates"),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "application_property_facets_ready",
            "property_facet_rows_after",
        ) => ("other-question", "managed property facets"),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "application_navigation_pages_ready",
            "navigation_pages_after",
        ) => ("other-question", "managed navigation pages"),
        // The §6.2 registry row source (P0-rust Wave D, `18f4265c`): the page map
        // and the property rows, read under ONE projection snapshot so a row
        // naming a page the map lacks is a consistency defect rather than a
        // silent Markdown fallback.
        (DIRECT, "property_owner_rows", "navigation_pages_after_with_header_validation") => {
            ("other-question", "Direct registry snapshot page map")
        }
        (DIRECT, "property_owner_rows", "property_facet_rows_after") => {
            ("other-question", "Direct registry snapshot property rows")
        }
        (
            "crates/tine-core/src/sync_runtime.rs",
            "application_property_registry_ready",
            "property_facet_rows_after",
        ) => ("other-question", "managed registry snapshot property rows"),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "application_property_registry_ready",
            "navigation_pages_after",
        ) => ("other-question", "managed registry snapshot page map"),
        _ => panic!("unclassified SQL read-family call: {file}::{symbol} {family}"),
    }
}

fn census(source: &BTreeMap<String, String>) -> BTreeSet<CensusRecord> {
    let families = [
        "navigation_pages_after_with_header_validation",
        "task_candidate_pages_after",
        "page_referrer_candidates_after",
        "block_property_candidates_after",
        "property_facet_rows_after",
        "navigation_pages_after",
    ];
    let mut records = BTreeSet::new();
    for (file, text) in source {
        for family in families {
            let pattern = Regex::new(&format!(r"\.\s*{}\s*\(", regex::escape(family))).unwrap();
            for found in pattern.find_iter(text) {
                let symbol = containing_symbol(text, found.start());
                let (class, question) = classify(file, &symbol, family);
                assert!(
                    records.insert(CensusRecord {
                        family: family.into(),
                        file: file.clone(),
                        enclosing_symbol: symbol,
                        call_expression: format!(".{family}("),
                        class: class.into(),
                        question: question.into(),
                    }),
                    "duplicate census record needs a more exact enclosing symbol"
                );
            }
        }
    }
    records
}

fn assert_exact_census(source: &BTreeMap<String, String>, expected: &BTreeSet<CensusRecord>) {
    assert_eq!(census(source), *expected, "I-12: the shared SQL read-family producer/consumer census changed; classify the exact enclosing production symbol and question; exemplar {LOWERING}");
}

fn expected_census() -> BTreeSet<CensusRecord> {
    let mut records = BTreeSet::new();
    let mut add = |family: &str, file: &str, symbol: &str, class: &str, question: &str| {
        assert!(records.insert(CensusRecord {
            family: family.into(),
            file: file.into(),
            enclosing_symbol: symbol.into(),
            call_expression: format!(".{family}("),
            class: class.into(),
            question: question.into(),
        }));
    };
    for owner in [
        "SqliteMaterializedRead<'_>",
        "SqliteGraphProjectionRead<'_>",
    ] {
        for (symbol, family) in [
            ("task_candidate_pages", "task_candidate_pages_after"),
            ("page_referrer_candidates", "page_referrer_candidates_after"),
            (
                "block_property_candidates",
                "block_property_candidates_after",
            ),
            ("page_property_candidates", "property_facet_rows_after"),
            (
                "navigation_pages",
                if owner.starts_with("SqliteMaterialized") {
                    "navigation_pages_after"
                } else {
                    "navigation_pages_after_with_header_validation"
                },
            ),
        ] {
            add(
                family,
                LOWERING,
                &format!("{owner}::{symbol}"),
                "shared-lowering-adapter",
                "SimpleQueryCandidatePlan page candidates",
            );
        }
    }
    for family in [
        "navigation_pages_after_with_header_validation",
        "page_referrer_candidates_after",
        "block_property_candidates_after",
        "property_facet_rows_after",
        "task_candidate_pages_after",
    ] {
        let symbol = match family {
            "navigation_pages_after_with_header_validation" => "navigation_pages_after",
            other => other,
        };
        add(
            family,
            "crates/tine-core/src/oplog/sqlite_materialization.rs",
            symbol,
            "facade-forwarder",
            "managed-to-physical read facade conversion",
        );
    }
    for (family, file, symbol, question) in [
        (
            "property_facet_rows_after",
            DIRECT,
            "property_facets",
            "Direct property facets",
        ),
        (
            "navigation_pages_after_with_header_validation",
            DIRECT,
            "real_page_names",
            "Direct real page ownership",
        ),
        // The §6.2 registry row source (P0-rust Wave D, `18f4265c`). It reads the
        // page map and the property rows under ONE projection snapshot, so it is
        // two classified reads in one symbol, not a new read family. CLOSURE §4
        // rejected answering this from the document walk: the walk aggregates
        // owner identity away, so it cannot report cardinality or distinct owners.
        (
            "navigation_pages_after_with_header_validation",
            DIRECT,
            "property_owner_rows",
            "Direct registry snapshot page map",
        ),
        (
            "property_facet_rows_after",
            DIRECT,
            "property_owner_rows",
            "Direct registry snapshot property rows",
        ),
        (
            "property_facet_rows_after",
            "crates/tine-core/src/sync_runtime.rs",
            "application_property_registry_ready",
            "managed registry snapshot property rows",
        ),
        (
            "navigation_pages_after",
            "crates/tine-core/src/sync_runtime.rs",
            "application_property_registry_ready",
            "managed registry snapshot page map",
        ),
        (
            "page_referrer_candidates_after",
            DIRECT,
            "reference_candidate_paths",
            "Direct explicit reference candidates",
        ),
        (
            "page_referrer_candidates_after",
            "crates/tine-core/src/sync_runtime.rs",
            "application_backlinks_ready",
            "managed backlinks",
        ),
        (
            "block_property_candidates_after",
            "crates/tine-core/src/sync_runtime.rs",
            "application_templates_ready",
            "managed templates",
        ),
        (
            "property_facet_rows_after",
            "crates/tine-core/src/sync_runtime.rs",
            "application_property_facets_ready",
            "managed property facets",
        ),
        (
            "navigation_pages_after",
            "crates/tine-core/src/sync_runtime.rs",
            "application_navigation_pages_ready",
            "managed navigation pages",
        ),
    ] {
        add(family, file, symbol, "other-question", question);
    }
    records
}

#[test]
fn simple_query_read_family_census_is_exact() {
    let source = sources();
    let raw_lowering = std::fs::read_to_string(repo_root().join(LOWERING)).unwrap();
    assert!(
        !raw_lowering.contains("source_to_sql_read_family_has_one_producer_file"),
        "I-11/I-12: remove the vacuous five-file co-occurrence guard and keep this whole-production-tree exact census"
    );
    assert_eq!(
        source[LOWERING]
            .matches("impl SimpleQuerySqlRead for ")
            .count(),
        2,
        "I-12: exactly the Managed and Direct adapters implement SimpleQuerySqlRead"
    );
    let expected = expected_census();
    assert_exact_census(&source, &expected);

    let representative = [
        ("task_candidate_pages_after", "Source::Task"),
        ("page_referrer_candidates_after", "Source::PageRef"),
        ("block_property_candidates_after", "Source::BlockProperty"),
        ("property_facet_rows_after", "Source::PageProperty"),
        ("navigation_pages_after", "Source::Page"),
        (
            "navigation_pages_after_with_header_validation",
            "Source::Journal",
        ),
    ];
    for (family, source_variant) in representative {
        let mut sixth_file = source.clone();
        sixth_file.insert(
            format!("crates/tine-core/src/rogue_{family}.rs"),
            format!("fn rogue(read: &Read) {{ read.{family}(None, 1); /* {source_variant} */ }}"),
        );
        assert!(std::panic::catch_unwind(|| assert_exact_census(&sixth_file, &expected)).is_err());

        let mut wrong_symbol = source.clone();
        wrong_symbol
            .get_mut("crates/tine-core/src/model.rs")
            .unwrap()
            .push_str(&format!(
                "\nfn rogue_{family}(read: &Read) {{ read.{family}(None, 1); }}\n"
            ));
        assert!(
            std::panic::catch_unwind(|| assert_exact_census(&wrong_symbol, &expected)).is_err()
        );

        let mut swapped = source.clone();
        let owner = expected
            .iter()
            .find(|record| record.family == family)
            .unwrap();
        let text = swapped.get_mut(&owner.file).unwrap();
        let needle = format!(".{family}(");
        let at = text.find(&needle).unwrap();
        text.replace_range(at..at + needle.len(), ".removed_allowed_call(");
        swapped
            .get_mut("crates/tine-core/src/model.rs")
            .unwrap()
            .push_str(&format!(
                "\nfn swapped_{family}(read: &Read) {{ read.{family}(None, 1); }}\n"
            ));
        assert!(std::panic::catch_unwind(|| assert_exact_census(&swapped, &expected)).is_err());
    }
}
