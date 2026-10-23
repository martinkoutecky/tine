//! One integration executable; module names retain each source suite in test filters.

// Store cost/fault probes are process-global. Every case takes this lock so
// ordinary graph activity cannot contaminate a measurement in another suite.
// Child workers start a fresh executable and therefore have their own lock.
fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[path = "br3_page_open_before_ready.rs"]
mod br3_page_open_before_ready;

#[path = "c3s_sync_copy_base.rs"]
mod c3s_sync_copy_base;

#[path = "c3w_keep_both_ids.rs"]
mod c3w_keep_both_ids;

#[path = "c3w_rename_refs.rs"]
mod c3w_rename_refs;

#[path = "c3w_rename_unreadable.rs"]
mod c3w_rename_unreadable;

#[path = "c3w_union_pre.rs"]
mod c3w_union_pre;

#[path = "c3y_alias_separator.rs"]
mod c3y_alias_separator;

#[path = "c3y_long_names.rs"]
mod c3y_long_names;

#[path = "client.rs"]
mod client;

#[path = "config_writes.rs"]
mod config_writes;

#[path = "custom_css_edit.rs"]
mod custom_css_edit;

#[path = "f8_concord.rs"]
mod f8_concord;

#[path = "f8_duplicate_journal.rs"]
mod f8_duplicate_journal;

#[path = "f8e_live_conflict.rs"]
mod f8e_live_conflict;

#[path = "failed_read_guide.rs"]
mod failed_read_guide;

#[path = "failed_reads.rs"]
mod failed_reads;

#[path = "favorites_config.rs"]
mod favorites_config;

#[path = "gh332_invalid_config.rs"]
mod gh332_invalid_config;

#[path = "gh481_fileless_days.rs"]
mod gh481_fileless_days;

#[path = "gh623_trash_referenced.rs"]
mod gh623_trash_referenced;

#[path = "graph_verification.rs"]
mod graph_verification;

#[path = "guide_host_op.rs"]
mod guide_host_op;

#[path = "home_rename.rs"]
mod home_rename;

#[path = "host_moves.rs"]
mod host_moves;

#[path = "host_op_conflicts.rs"]
mod host_op_conflicts;

#[path = "i04_roundtrip.rs"]
mod i04_roundtrip;

#[path = "i12_single_definitions.rs"]
mod i12_single_definitions;

#[path = "i22_hostile_inputs.rs"]
mod i22_hostile_inputs;

#[path = "i22_macro_fanout.rs"]
mod i22_macro_fanout;

#[path = "i22_read_guard.rs"]
mod i22_read_guard;

#[path = "identity_switch_guide.rs"]
mod identity_switch_guide;

#[path = "journal_migrations.rs"]
mod journal_migrations;

#[path = "l8_03_duplicate_day_asset.rs"]
mod l8_03_duplicate_day_asset;

#[path = "legacy_config.rs"]
mod legacy_config;

#[path = "legacy_onboarding.rs"]
mod legacy_onboarding;

#[path = "legacy_publish.rs"]
mod legacy_publish;

#[path = "live_home.rs"]
mod live_home;

#[path = "og_c5_p2_bad_file.rs"]
mod og_c5_p2_bad_file;

#[path = "og_c5_p2_merge_rename.rs"]
mod og_c5_p2_merge_rename;

#[path = "og_dupal2_format.rs"]
mod og_dupal2_format;

#[path = "og_gh644_page_open.rs"]
mod og_gh644_page_open;

#[path = "og_qc3_onboarding.rs"]
mod og_qc3_onboarding;

#[path = "og_qd1_onboarding.rs"]
mod og_qd1_onboarding;

#[path = "og_qe3_journal_formats.rs"]
mod og_qe3_journal_formats;

#[path = "og_r_print_cost.rs"]
mod og_r_print_cost;

#[path = "og_r_render.rs"]
mod og_r_render;

#[path = "operation_claims_keep_rows.rs"]
mod operation_claims_keep_rows;

#[path = "os_handoff.rs"]
mod os_handoff;

#[path = "page_case_alias.rs"]
mod page_case_alias;

#[path = "page_delete_missing.rs"]
mod page_delete_missing;

#[path = "page_rename_identity.rs"]
mod page_rename_identity;

#[path = "page_rename_merge.rs"]
mod page_rename_merge;

#[path = "page_rename_touched.rs"]
mod page_rename_touched;

#[path = "pdf_area_lifecycle.rs"]
mod pdf_area_lifecycle;

#[path = "pdf_host.rs"]
mod pdf_host;

#[path = "perf3_vcs_anchor_flag.rs"]
mod perf3_vcs_anchor_flag;

#[path = "perf3_vcs_anchor_reads.rs"]
mod perf3_vcs_anchor_reads;

#[path = "print_publish.rs"]
mod print_publish;

#[path = "publish_selection.rs"]
mod publish_selection;

#[path = "query_owner_context.rs"]
mod query_owner_context;

#[path = "query_publication.rs"]
mod query_publication;

#[path = "rename_cost.rs"]
mod rename_cost;

#[path = "rename_cost_probe.rs"]
mod rename_cost_probe;

#[path = "rename_io.rs"]
mod rename_io;

#[path = "rename_marker_quarantine.rs"]
mod rename_marker_quarantine;

#[path = "rescue_identity.rs"]
mod rescue_identity;

#[path = "retained_host.rs"]
mod retained_host;

#[path = "retained_over_running_rename.rs"]
mod retained_over_running_rename;

#[path = "sheets_export.rs"]
mod sheets_export;

#[path = "sheets_export_chrome.rs"]
mod sheets_export_chrome;

#[path = "transport_flows.rs"]
mod transport_flows;
