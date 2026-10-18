//! One integration executable; module names retain each source suite in test filters.

// Store cost/fault probes are process-global. Every case takes this lock so
// ordinary graph activity cannot contaminate a measurement in another suite.
// Child workers start a fresh executable and therefore have their own lock.
fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[path = "../src/test_cfg.rs"]
pub(crate) mod test_cfg;

// Process-wide allocation probes must also exclude libtest's own thread and
// output bookkeeping, which runs outside integration_lock.
fn isolated_case(name: &str) -> bool {
    const CHILD: &str = "TINE_INTEGRATION_ALLOCATION_CASE";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, name)
        .output()
        .expect("start isolated allocation probe");
    assert!(
        output.status.success(),
        "allocation probe {name}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[path = "answer_changes.rs"]
mod answer_changes;

#[path = "asset_watch.rs"]
mod asset_watch;

#[path = "b_query.rs"]
mod b_query;

#[path = "b_query_guard.rs"]
mod b_query_guard;

#[path = "b_tail.rs"]
mod b_tail;

#[path = "b_tail_filename_guard.rs"]
mod b_tail_filename_guard;

#[path = "bench_events_off.rs"]
mod bench_events_off;

#[path = "c3s_content_loss.rs"]
mod c3s_content_loss;

#[path = "c3w_long_names.rs"]
mod c3w_long_names;

#[path = "c3y_derived_names_guard.rs"]
mod c3y_derived_names_guard;

#[path = "c3y_property_separator_guard.rs"]
mod c3y_property_separator_guard;

#[path = "client_root_boundary.rs"]
mod client_root_boundary;

#[path = "cold_search_cost.rs"]
mod cold_search_cost;

#[path = "config_live_reload.rs"]
mod config_live_reload;

#[path = "directory_durability_guard.rs"]
mod directory_durability_guard;

#[path = "f8_vcs_markers.rs"]
mod f8_vcs_markers;

#[path = "favorites_page_bytes.rs"]
mod favorites_page_bytes;

#[path = "favorites_reference_source.rs"]
mod favorites_reference_source;

#[path = "gh210_sheet_first_bullet.rs"]
mod gh210_sheet_first_bullet;

#[path = "gh540_numbered_first_bullet.rs"]
mod gh540_numbered_first_bullet;

#[path = "graph_command_boundary.rs"]
mod graph_command_boundary;

#[path = "host_edit_cost.rs"]
mod host_edit_cost;

#[path = "i01_write_protocol_census.rs"]
mod i01_write_protocol_census;

#[path = "i02_crash_gaps.rs"]
mod i02_crash_gaps;

#[path = "i05_log_sinks.rs"]
mod i05_log_sinks;

#[path = "i08_refusal_table.rs"]
mod i08_refusal_table;

#[path = "i12_page_header_golden.rs"]
mod i12_page_header_golden;

#[path = "i13_edit_cost.rs"]
mod i13_edit_cost;

#[path = "i16_platform_arms.rs"]
mod i16_platform_arms;

#[path = "i21_owners.rs"]
mod i21_owners;

#[path = "i22_advanced_query_depth.rs"]
mod i22_advanced_query_depth;

#[path = "i22_deep_directories.rs"]
mod i22_deep_directories;

#[path = "i22_property_value_depth.rs"]
mod i22_property_value_depth;

#[path = "journal_sections_locate.rs"]
mod journal_sections_locate;

#[path = "launch_one_read.rs"]
mod launch_one_read;

#[path = "lifecycle.rs"]
mod lifecycle;

#[path = "linkable_page_property_refs.rs"]
mod linkable_page_property_refs;

#[path = "lone_cr_pages.rs"]
mod lone_cr_pages;

#[path = "og_b_cost.rs"]
mod og_b_cost;

#[path = "og_b_cost_names.rs"]
mod og_b_cost_names;

#[path = "og_b_door2.rs"]
mod og_b_door2;

#[path = "og_b_store_config.rs"]
mod og_b_store_config;

#[path = "og_b_store_cost.rs"]
mod og_b_store_cost;

#[path = "og_b_store_export.rs"]
mod og_b_store_export;

#[path = "og_b_store_journal.rs"]
mod og_b_store_journal;

#[path = "og_b_store_move.rs"]
mod og_b_store_move;

#[path = "og_c5q_alias_phantom.rs"]
mod og_c5q_alias_phantom;

#[path = "og_c5q_false_refusals.rs"]
mod og_c5q_false_refusals;

#[path = "og_create_unreadable_owner.rs"]
mod og_create_unreadable_owner;

#[path = "og_diag_dump.rs"]
mod og_diag_dump;

#[path = "og_differential_rename.rs"]
mod og_differential_rename;

#[path = "og_differential_rename_merge.rs"]
mod og_differential_rename_merge;

#[path = "og_gh609_case_rename.rs"]
mod og_gh609_case_rename;

#[path = "og_gh651_nested_task_rows.rs"]
mod og_gh651_nested_task_rows;

#[path = "og_k1_guards.rs"]
mod og_k1_guards;

#[path = "og_k1_transaction.rs"]
mod og_k1_transaction;

#[path = "og_p10_journal_scan_cost.rs"]
mod og_p10_journal_scan_cost;

#[path = "og_q617_page_property_blocks.rs"]
mod og_q617_page_property_blocks;

#[path = "og_q617b_query_semantics.rs"]
mod og_q617b_query_semantics;

#[path = "og_qd8_query_decisions.rs"]
mod og_qd8_query_decisions;

#[path = "og_r3b_native_remainders.rs"]
mod og_r3b_native_remainders;

#[path = "og_r5_guard.rs"]
mod og_r5_guard;

#[path = "og_r5_publication.rs"]
mod og_r5_publication;

#[path = "og_r5_rename_cost.rs"]
mod og_r5_rename_cost;

#[path = "og_r5_save_parse.rs"]
mod og_r5_save_parse;

#[path = "org_headline_refs.rs"]
mod org_headline_refs;

#[path = "page_title_identity.rs"]
mod page_title_identity;

#[path = "perf3_asset_walk_stamps.rs"]
mod perf3_asset_walk_stamps;

#[path = "publish_external_kill.rs"]
mod publish_external_kill;

#[path = "publish_site.rs"]
mod publish_site;

#[path = "query_conformance.rs"]
mod query_conformance;

#[path = "query_export_discriminator.rs"]
mod query_export_discriminator;

#[path = "query_gh542.rs"]
mod query_gh542;

#[path = "query_publication_commit.rs"]
mod query_publication_commit;

#[path = "query_scoped_search.rs"]
mod query_scoped_search;

#[path = "query_unit_cost.rs"]
mod query_unit_cost;

#[path = "query_unreadable_candidate.rs"]
mod query_unreadable_candidate;

#[path = "racy_delivery_cost.rs"]
mod racy_delivery_cost;

#[path = "restore.rs"]
mod restore;

#[path = "save_layout_retention.rs"]
mod save_layout_retention;

#[path = "scan_area.rs"]
mod scan_area;

#[path = "shallow_ratchet.rs"]
mod shallow_ratchet;

#[path = "store_read.rs"]
mod store_read;

#[path = "store_save.rs"]
mod store_save;

#[path = "transaction.rs"]
mod transaction;

#[path = "transaction_plan.rs"]
mod transaction_plan;

#[path = "watch.rs"]
mod watch;

#[path = "whole_graph.rs"]
mod whole_graph;
