//! Test-only evidence for GitHub issue #137. No production behavior is changed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use tine_core::model::ReferenceKind;
use tine_core::{Graph, PageKind, RefGroup};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let ordinal = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tine-issue137-{label}-{}-{ordinal}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::create_dir_all(root.join("logseq/bak")).unwrap();
        Self { root }
    }

    fn page(&self, name: &str, body: &str) {
        fs::write(self.root.join("pages").join(format!("{name}.md")), body).unwrap();
    }

    fn nested_page(&self, dir: &str, name: &str, body: &str) {
        let path = self.root.join("pages").join(dir);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(format!("{name}.md")), body).unwrap();
    }

    fn graph(&self) -> Graph {
        Graph::open(&self.root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn row_count(groups: &[RefGroup]) -> usize {
    groups.iter().map(|group| group.blocks.len()).sum()
}

fn source_pages(groups: &[RefGroup]) -> Vec<String> {
    groups.iter().map(|group| group.page.clone()).collect()
}

fn evidence_count(groups: &[RefGroup], kind: ReferenceKind) -> usize {
    groups
        .iter()
        .flat_map(|group| group.evidence.iter())
        .flat_map(|evidence| evidence.occurrences.iter())
        .filter(|occurrence| occurrence.kind == kind)
        .count()
}

fn membership_signature(groups: &[RefGroup]) -> Vec<String> {
    groups
        .iter()
        .map(|group| {
            let blocks = group
                .blocks
                .iter()
                .map(|block| block.raw.as_str())
                .collect::<Vec<_>>()
                .join("\u{1f}");
            let occurrences = group
                .evidence
                .iter()
                .flat_map(|evidence| evidence.occurrences.iter())
                .map(|occurrence| {
                    format!(
                        "{}:{:?}:{}-{}:{}",
                        occurrence.matched_name,
                        occurrence.kind,
                        occurrence.span.start,
                        occurrence.span.end,
                        occurrence.rule
                    )
                })
                .collect::<Vec<_>>()
                .join("\u{1e}");
            format!("{}:{:?}:{blocks}:{occurrences}", group.page, group.kind)
        })
        .collect()
}

#[test]
fn issue137_current_contract_snapshot_uses_real_parser_and_engine() {
    let fixture = Fixture::new("contract");
    fixture.page("Target", "alias:: Alias\n\n- target page\n");
    fixture.page(
        "Source",
        concat!(
            "- explicit title [[Target]]\n",
            "- plain title Target\n",
            "- explicit alias [[Alias]]\n",
            "- plain alias Alias\n",
            "- mixed [[Target]] then Target\n",
            "- repeated Target then Target\n",
            "- inline `Target` then visible Target\n",
            "- escaped \\[[Target]]\n",
            "- punctuation (Target), [Target], _Target_\n",
            "- continuous 北京Target上海\n",
            "- ascii neighbors aTargetz 1Target2\n",
            "- markdown [Target](https://example.invalid/)\n",
            "- custom:: Target\n",
            "- id:: 11111111-1111-4111-8111-111111111111\n",
            "- ((11111111-1111-4111-8111-111111111111))\n",
            "- ```text\n  Target\n  [[Target]]\n  ```\n",
        ),
    );
    fixture.page("Props", "related:: [[Target]]\nplain:: Target\n\n- body\n");
    fs::write(
        fixture.root.join("logseq/bak/Excluded.md"),
        "- [[Target]] Target\n",
    )
    .unwrap();

    let graph = fixture.graph();
    let cold_linked = graph.backlinks("Target");
    let cold_unlinked = graph.unlinked_refs("Target");
    let cold_linked_json = serde_json::to_string(cold_linked.as_ref()).unwrap();
    let cold_unlinked_json = serde_json::to_string(cold_unlinked.as_ref()).unwrap();
    graph.warm_cache();
    assert_eq!(
        cold_linked_json,
        serde_json::to_string(graph.backlinks("Target").as_ref()).unwrap()
    );
    assert_eq!(
        cold_unlinked_json,
        serde_json::to_string(graph.unlinked_refs("Target").as_ref()).unwrap()
    );
    graph.invalidate_cache();
    assert_eq!(
        membership_signature(cold_linked.as_ref()),
        membership_signature(graph.backlinks("Target").as_ref())
    );
    assert_eq!(
        membership_signature(cold_unlinked.as_ref()),
        membership_signature(graph.unlinked_refs("Target").as_ref())
    );
    assert!(!source_pages(cold_linked.as_ref())
        .iter()
        .any(|page| page == "Excluded"));
    assert!(!source_pages(cold_unlinked.as_ref())
        .iter()
        .any(|page| page == "Excluded"));

    let diagnostics = graph.reference_diagnostics("Target");
    println!(
        "ISSUE137_CONTRACT_TRACE={} ",
        serde_json::to_string_pretty(&diagnostics).unwrap()
    );
    println!(
        "ISSUE137_CONTRACT_COUNTS=linked_rows:{} linked_occurrences:{} unlinked_rows:{} unlinked_occurrences:{}",
        row_count(cold_linked.as_ref()),
        evidence_count(cold_linked.as_ref(), ReferenceKind::Explicit),
        row_count(cold_unlinked.as_ref()),
        evidence_count(cold_unlinked.as_ref(), ReferenceKind::Plain),
    );
}

#[test]
fn issue232_cache_rebuild_preserves_block_identity() {
    let fixture = Fixture::new("cache-identity");
    fixture.page("Target", "- target\n");
    fixture.page("Source", "- [[Target]] then Target\n");
    let graph = fixture.graph();
    let first = serde_json::to_string(graph.backlinks("Target").as_ref()).unwrap();
    graph.warm_cache();
    assert_eq!(
        first,
        serde_json::to_string(graph.backlinks("Target").as_ref()).unwrap()
    );
    graph.invalidate_cache();
    assert_eq!(
        first,
        serde_json::to_string(graph.backlinks("Target").as_ref()).unwrap()
    );
}

#[test]
fn issue232_cold_page_cache_and_reference_rows_share_runtime_ids() {
    let fixture = Fixture::new("cold-cache-reference-identity");
    fixture.page("Target", "- target\n");
    fixture.page("Source", "- [[Target]] then Target\n");
    let graph = fixture.graph();
    let source = graph.find_entry("Source", PageKind::Page).unwrap();

    let cold_id = graph.load_page(&source).unwrap().blocks[0].id.clone();
    let linked = graph.backlinks("Target");
    let linked_source = linked.iter().find(|group| group.page == "Source").unwrap();
    assert_eq!(linked_source.blocks[0].id, cold_id);
    assert_eq!(linked_source.evidence[0].block_id, cold_id);
    let unlinked = graph.unlinked_refs("Target");
    let unlinked_source = unlinked
        .iter()
        .find(|group| group.page == "Source")
        .unwrap();
    assert_eq!(unlinked_source.blocks[0].id, cold_id);
    assert_eq!(graph.load_page(&source).unwrap().blocks[0].id, cold_id);
}

#[test]
fn issue232_runtime_ids_distinguish_structure_and_physical_owner() {
    const UNIQUE: &str = "11111111-1111-4111-8111-111111111111";
    const DUPLICATE: &str = "22222222-2222-4222-8222-222222222222";
    let fixture = Fixture::new("structural-owner-identity");
    fixture.page(
        "Identity",
        &format!(
            "- same\n- same\n- unique\n  id:: {UNIQUE}\n- duplicate one\n  id:: {DUPLICATE}\n- duplicate two\n  id:: {DUPLICATE}\n"
        ),
    );
    fixture.nested_page("client-a", "Foo", "- same\n");
    fixture.nested_page("client-b", "Foo", "- same\n");
    let graph = fixture.graph();

    let identity = graph.load_by_path("pages/Identity.md").unwrap().unwrap();
    let ids = identity
        .blocks
        .iter()
        .map(|block| block.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), identity.blocks.len());
    assert_ne!(identity.blocks[0].id, identity.blocks[1].id);
    assert_ne!(identity.blocks[2].id, UNIQUE);
    assert_ne!(identity.blocks[3].id, identity.blocks[4].id);
    assert_ne!(identity.blocks[3].id, DUPLICATE);
    assert_ne!(identity.blocks[4].id, DUPLICATE);

    let resolved = graph.resolve_block(UNIQUE).unwrap();
    assert_eq!(resolved.blocks[0].raw.lines().next(), Some("unique"));
    assert_eq!(resolved.blocks[0].id, identity.blocks[2].id);

    let a = graph
        .load_by_path("pages/client-a/Foo.md")
        .unwrap()
        .unwrap();
    let b = graph
        .load_by_path("pages/client-b/Foo.md")
        .unwrap()
        .unwrap();
    assert_ne!(a.blocks[0].id, b.blocks[0].id);
}

#[test]
fn issue232_merge_output_matches_destination_reload_identity() {
    let fixture = Fixture::new("merge-destination-identity");
    fixture.page("Source", "- moved one\n- moved two\n");
    fixture.page("Destination", "- kept\n");
    let graph = fixture.graph();
    graph.warm_cache();
    let destination = graph
        .find_entry("Destination", PageKind::Page)
        .unwrap();

    graph
        .merge_pages("pages/Source.md", "pages/Destination.md")
        .unwrap();
    let merged_ids = graph
        .load_page(&destination)
        .unwrap()
        .blocks
        .into_iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    assert_eq!(merged_ids.len(), 3);

    graph.invalidate_cache();
    let reloaded_ids = graph
        .load_page(&destination)
        .unwrap()
        .blocks
        .into_iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    assert_eq!(merged_ids, reloaded_ids);
}

#[test]
fn issue232_runtime_ids_never_serialize_as_synthetic_id_properties() {
    let fixture = Fixture::new("runtime-id-not-serialized");
    let original = "- first\n\t- child\n- second\n";
    fixture.page("Roundtrip", original);
    let graph = fixture.graph();
    let entry = graph.find_entry("Roundtrip", PageKind::Page).unwrap();
    let dto = graph.load_page(&entry).unwrap();
    assert!(dto.blocks.iter().all(|block| !block.id.is_empty()));

    graph.save_page(&dto, dto.rev.as_deref()).unwrap();
    let persisted = fs::read_to_string(fixture.root.join("pages/Roundtrip.md")).unwrap();
    assert_eq!(persisted, original);
    assert!(!persisted.contains("id::"));
}

#[test]
fn issue137_fail_before_nfc_nfd_equivalence() {
    let fixture = Fixture::new("nfc-nfd");
    fixture.page("Café", "- target\n");
    fixture.page("Source", "- [[Cafe\u{301}]] and Cafe\u{301}\n");
    let graph = fixture.graph();
    assert_eq!(
        (
            source_pages(graph.backlinks("Café").as_ref()),
            source_pages(graph.unlinked_refs("Café").as_ref())
        ),
        (vec!["Source".to_string()], vec!["Source".to_string()])
    );
}

#[test]
fn issue137_fail_before_real_title_wins_alias_collision() {
    let fixture = Fixture::new("title-alias-collision");
    fixture.page("Shared", "- real page self-link [[Shared]] and Shared\n");
    fixture.page(
        "Alias Owner",
        "alias:: Shared\n\n- alias owner links [[Shared]] and says Shared\n",
    );
    fixture.page("Source", "- [[Shared]] then Shared\n");
    let graph = fixture.graph();

    assert_eq!(
        source_pages(graph.backlinks("Shared").as_ref()),
        vec!["Alias Owner", "Source"]
    );
    assert_eq!(
        source_pages(graph.unlinked_refs("Shared").as_ref()),
        vec!["Alias Owner", "Source"]
    );
    // OG's bidirectional/transitive `:alias` rule and `page-alias-set` make
    // every component member see the Source evidence for `[[Shared]]`.
    let owner_linked = graph.backlinks("Alias Owner");
    let owner_unlinked = graph.unlinked_refs("Alias Owner");
    assert!(source_pages(owner_linked.as_ref()).contains(&"Source".to_string()));
    assert!(source_pages(owner_unlinked.as_ref()).contains(&"Source".to_string()));
    assert!(owner_linked
        .iter()
        .flat_map(|group| group.blocks.iter())
        .any(|block| block.raw.contains("[[Shared]]")));
}

#[test]
fn issue137_fail_before_uuid_property_is_not_a_page_mention() {
    const UUID: &str = "11111111-1111-4111-8111-111111111111";
    let fixture = Fixture::new("uuid-property");
    fixture.page(UUID, "- target\n");
    fixture.page(
        "Source",
        &format!("- metadata only\n  id:: {UUID}\n- (({UUID}))\n"),
    );
    let graph = fixture.graph();
    assert!(graph.unlinked_refs(UUID).is_empty());
    assert!(graph.backlinks(UUID).is_empty());
}

#[test]
fn issue137_fail_before_occurrence_count_is_not_silently_truncated() {
    let fixture = Fixture::new("occurrence-cap");
    fixture.page("Target", "- target\n");
    fixture.page("Source", &format!("- {}\n", "Target ".repeat(65)));
    let graph = fixture.graph();
    let groups = graph.unlinked_refs("Target");
    assert_eq!(row_count(groups.as_ref()), 1);
    let evidence = &groups[0].evidence[0];
    assert_eq!(evidence.total, 65);
    assert_eq!(evidence.occurrences.len(), 64);
    assert!(evidence.truncated);
}

#[test]
fn issue137_page_and_block_property_asymmetry_witness() {
    let fixture = Fixture::new("property-asymmetry");
    fixture.page("Target", "- target\n");
    fixture.page("Page Property", "custom:: Target\n\n- body\n");
    fixture.page("Block Property", "- body\n  custom:: Target\n");
    let graph = fixture.graph();
    let pages = source_pages(graph.unlinked_refs("Target").as_ref());
    assert_eq!(pages, vec!["Block Property", "Page Property"]);
}

#[test]
fn issue137_fail_before_diagnostic_matches_backend_membership() {
    let fixture = Fixture::new("diagnostic-backend-parity");
    fixture.page("Target", "- target\n");
    fixture.page("Page Property", "custom:: Target\n\n- body\n");
    let graph = fixture.graph();

    let in_backend = graph
        .unlinked_refs("Target")
        .iter()
        .any(|group| group.page == "Page Property");
    let trace = graph
        .reference_diagnostics("Target")
        .traces
        .into_iter()
        .find(|trace| trace.page == "Page Property")
        .expect("diagnostic should expose the textual candidate");
    assert_eq!(trace.included_unlinked, in_backend);
}

#[test]
fn issue137_distinct_blocks_and_nested_directories_remain_distinct_rows() {
    let fixture = Fixture::new("distinct-spans");
    fixture.page("Target", "- target\n");
    fixture.nested_page("a", "One", "- Target\n- Target\n");
    fixture.nested_page("b", "Two", "- Target\n");
    let graph = fixture.graph();
    let groups = graph.unlinked_refs("Target");
    assert_eq!(row_count(groups.as_ref()), 3);
    assert_eq!(evidence_count(groups.as_ref(), ReferenceKind::Plain), 3);
}

#[test]
fn issue137_core_preserves_same_basename_source_groups_before_ui() {
    let fixture = Fixture::new("same-basename-groups");
    fixture.page("Target", "- target\n");
    fixture.nested_page("a", "Duplicate", "- [[Target]] from A\n");
    fixture.nested_page("b", "Duplicate", "- [[Target]] from B\n");
    let graph = fixture.graph();
    let groups = graph.backlinks("Target");
    assert_eq!(groups.len(), 1);
    assert_eq!(row_count(groups.as_ref()), 2);
    assert!(groups.iter().all(|group| group.page == "Duplicate"));
    let raws = groups
        .iter()
        .flat_map(|group| group.blocks.iter().map(|block| block.raw.as_str()))
        .collect::<Vec<_>>();
    assert!(raws.iter().any(|raw| raw.contains("from A")));
    assert!(raws.iter().any(|raw| raw.contains("from B")));
}

#[test]
fn issue137_referenced_only_target_is_queryable() {
    let fixture = Fixture::new("referenced-only");
    fixture.page("Source", "- [[Ghost Page]] and Ghost Page\n");
    let graph = fixture.graph();
    assert_eq!(
        source_pages(graph.backlinks("Ghost Page").as_ref()),
        vec!["Source"]
    );
    assert_eq!(
        source_pages(graph.unlinked_refs("Ghost Page").as_ref()),
        vec!["Source"]
    );
    assert!(graph.find_entry("Ghost Page", PageKind::Page).is_none());
}

#[test]
fn issue137_creation_order_does_not_change_non_colliding_membership() {
    fn build(label: &str, reverse: bool) -> Vec<String> {
        let fixture = Fixture::new(label);
        fixture.page("Target", "- target\n");
        let pages = [("Alpha", "- Target\n"), ("Beta", "- Target\n")];
        for (name, body) in if reverse {
            pages.into_iter().rev().collect::<Vec<_>>()
        } else {
            pages.into_iter().collect::<Vec<_>>()
        } {
            fixture.page(name, body);
        }
        source_pages(fixture.graph().unlinked_refs("Target").as_ref())
    }
    assert_eq!(build("order-forward", false), build("order-reverse", true));
}

#[test]
fn issue137_generated_boundary_and_classification_metamorphics() {
    let fixture = Fixture::new("generated-boundaries");
    fixture.page("Target", "- target\n");
    let allowed = [
        ("Cjk", "- 北京Target上海\n"),
        ("Underscore", "- _Target_\n"),
        ("Accent", "- éTargeté\n"),
        ("Punctuation", "- (Target), +Target+\n"),
        ("Mixed", "- English Target 中文\n"),
    ];
    let rejected = [
        ("AsciiLetters", "- aTargetz\n"),
        ("AsciiDigits", "- 1Target2\n"),
        ("MixedAdjacent", "- 中文TargetEnglish\n"),
    ];
    for (name, body) in allowed.into_iter().chain(rejected) {
        fixture.page(name, body);
    }

    let graph = fixture.graph();
    assert_eq!(
        source_pages(graph.unlinked_refs("Target").as_ref()),
        vec!["Accent", "Cjk", "Mixed", "Punctuation", "Underscore"]
    );
    assert!(graph.backlinks("Target").is_empty());

    let plain = Fixture::new("wrap-plain");
    plain.page("Target", "- target\n");
    plain.page("Source", "- Target\n");
    assert_eq!(row_count(plain.graph().unlinked_refs("Target").as_ref()), 1);

    let explicit = Fixture::new("wrap-explicit");
    explicit.page("Target", "- target\n");
    explicit.page("Source", "- [[Target]]\n");
    let explicit_graph = explicit.graph();
    assert_eq!(row_count(explicit_graph.backlinks("Target").as_ref()), 1);
    assert_eq!(
        row_count(explicit_graph.unlinked_refs("Target").as_ref()),
        0
    );
}

#[test]
fn issue137_generated_character_classes_preserve_boundary_contract() {
    let fixture = Fixture::new("generated-character-classes");
    fixture.page("Target", "- target\n");

    let allowed_neighbors = (0x4e00..0x5000)
        .step_by(97)
        .filter_map(char::from_u32)
        .chain(['_', 'é', '—', '（', '）']);
    let mut allowed_count = 0;
    for (index, neighbor) in allowed_neighbors.enumerate() {
        fixture.page(
            &format!("Allowed {index:03}"),
            &format!("- {neighbor}Target{neighbor}\n"),
        );
        allowed_count += 1;
    }
    let rejected_neighbors = ('a'..='z').chain('A'..='Z').chain('0'..='9');
    for (index, neighbor) in rejected_neighbors.enumerate() {
        fixture.page(
            &format!("Rejected {index:03}"),
            &format!("- {neighbor}Target{neighbor}\n"),
        );
    }

    let pages = source_pages(fixture.graph().unlinked_refs("Target").as_ref());
    assert_eq!(pages.len(), allowed_count);
    assert!(pages.iter().all(|page| page.starts_with("Allowed ")));
}

#[test]
fn issue137_many_unrelated_matches_do_not_apply_a_hidden_top_n() {
    let fixture = Fixture::new("many-matches");
    fixture.page("Target", "- target\n");
    for index in 0..256 {
        fixture.page(&format!("Source {index:03}"), "- Target\n");
    }
    let graph = fixture.graph();
    let groups = graph.unlinked_refs("Target");
    assert_eq!(row_count(groups.as_ref()), 256);
    assert_eq!(evidence_count(groups.as_ref(), ReferenceKind::Plain), 256);
    assert!(source_pages(groups.as_ref()).contains(&"Source 255".to_string()));
}

#[test]
fn issue137_duplicate_alias_resolution_trace() {
    fn build(label: &str, swap_directories: bool) -> String {
        let fixture = Fixture::new(label);
        let owners = if swap_directories {
            [("a", "Owner B"), ("z", "Owner A")]
        } else {
            [("a", "Owner A"), ("z", "Owner B")]
        };
        for (dir, owner) in owners {
            fixture.nested_page(dir, owner, "alias:: Shared\n\n- owner\n");
        }
        fixture.page("Source", "- [[Shared]] Shared\n");
        fixture.graph().reference_diagnostics("Shared").target
    }
    let forward = build("duplicate-alias-forward", false);
    let reverse = build("duplicate-alias-reverse", true);
    println!("ISSUE137_DUPLICATE_ALIAS_TARGETS=forward:{forward:?} reverse:{reverse:?}");
    assert_eq!(forward, "Owner A");
    assert_eq!(reverse, "Owner A");
}

#[test]
fn issue137_fail_before_duplicate_alias_is_deterministic() {
    fn target(label: &str, swap_directories: bool) -> String {
        let fixture = Fixture::new(label);
        let owners = if swap_directories {
            [("a", "Owner B"), ("z", "Owner A")]
        } else {
            [("a", "Owner A"), ("z", "Owner B")]
        };
        for (dir, owner) in owners {
            fixture.nested_page(dir, owner, "alias:: Shared\n\n- owner\n");
        }
        fixture.graph().reference_diagnostics("Shared").target
    }
    assert_eq!(
        target("duplicate-alias-layout-a", false),
        target("duplicate-alias-layout-b", true)
    );
}

#[test]
fn issue137_alias_component_is_transitive_and_unions_duplicate_owners() {
    let transitive = Fixture::new("alias-transitive");
    transitive.page("A", "alias:: B\n\n- A\n");
    transitive.page("B", "alias:: C\n\n- B\n");
    transitive.page("C", "- C\n");
    transitive.page("Transitive Source", "- [[C]] then C\n");
    let graph = transitive.graph();
    assert!(source_pages(graph.backlinks("A").as_ref()).contains(&"Transitive Source".to_string()));
    assert!(
        source_pages(graph.unlinked_refs("A").as_ref()).contains(&"Transitive Source".to_string())
    );

    fn duplicate_component(label: &str, swap_directories: bool) -> (Vec<String>, Vec<String>) {
        let fixture = Fixture::new(label);
        let owners = if swap_directories {
            [("a", "Owner B"), ("z", "Owner A")]
        } else {
            [("a", "Owner A"), ("z", "Owner B")]
        };
        for (dir, owner) in owners {
            fixture.nested_page(dir, owner, "alias:: Shared\n\n- owner\n");
        }
        fixture.page("Witness A", "- [[Owner A]] then Owner A\n");
        fixture.page("Witness B", "- [[Owner B]] then Owner B\n");
        let graph = fixture.graph();
        (
            source_pages(graph.backlinks("Shared").as_ref()),
            source_pages(graph.unlinked_refs("Shared").as_ref()),
        )
    }

    let forward = duplicate_component("alias-union-forward", false);
    let reverse = duplicate_component("alias-union-reverse", true);
    assert_eq!(forward, reverse);
    for witness in ["Witness A", "Witness B"] {
        assert!(forward.0.contains(&witness.to_string()));
        assert!(forward.1.contains(&witness.to_string()));
    }
}

fn _assert_paths_are_private_fixture_only(path: &Path) {
    assert!(path.starts_with(std::env::temp_dir()));
}
