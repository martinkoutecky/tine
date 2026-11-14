//! One integration executable; module names retain each source suite in test filters.

mod comment_surfaces;
#[path = "../../test-support/temp_dir.rs"]
mod test_support;

use tine_store as store_crate;
#[path = "../../tine-store/src/test_host_writer.rs"]
mod test_host_writer;

// Store cost probes are process-global: the cost counters and the process's
// thread, file-descriptor and allocation totals. A case that reads them takes
// `exclusive_integration_lock`, so no other case's graph activity runs beside
// its measurement. Every other case takes the shared `integration_lock` and
// runs in parallel with the rest (scripts/test-rust-integration-layout.mjs
// checks that every case takes one). Fault plants are per thread
// (`host_faults`), so they need no lock.
// Child workers start a fresh executable and therefore have their own lock.
static INTEGRATION_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

fn integration_lock() -> std::sync::RwLockReadGuard<'static, ()> {
    INTEGRATION_LOCK
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn exclusive_integration_lock() -> std::sync::RwLockWriteGuard<'static, ()> {
    INTEGRATION_LOCK
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `store`'s static site as `tine publish` writes it (`publish_static`, rendered
/// from the private build graph): the folder it landed in and its page count.
/// Each call writes a fresh folder outside the graph.
fn published_site(store: &tine_store::Store) -> std::io::Result<(String, usize)> {
    // Callers read the output after return. Keep that lifetime, with a fresh
    // parent even when the OS reuses a test process's PID.
    let parent = tempfile::Builder::new()
        .prefix("tine-published-site-")
        .tempdir()?
        .keep();
    let receipt = tine_graph_features::publish_query::publish_static(
        store,
        &parent,
        "site",
        store.config().all_pages_public,
        true,
    )?;
    Ok((receipt.path, receipt.pages))
}

/// A running page host for `store`, the only writer of page files
/// (host-only-writer DESIGN §3.6), under its own app data; it derefs to the
/// host, so `&test_host(&store)` is a writer's `&PageHost`. Drop it before
/// removing the graph.
struct TestHost {
    host: tine_store::PageHost,
    _app_data: tempfile::TempDir,
}

impl std::ops::Deref for TestHost {
    type Target = tine_store::PageHost;
    fn deref(&self) -> &tine_store::PageHost {
        &self.host
    }
}

fn test_host(store: &std::sync::Arc<tine_store::Store>) -> TestHost {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    TestHost {
        host,
        _app_data: app_data,
    }
}

#[path = "br3_page_open_before_ready.rs"]
mod br3_page_open_before_ready;

#[path = "audit_rn_b.rs"]
mod audit_rn_b;

#[path = "c3s_sync_copy_base.rs"]
mod c3s_sync_copy_base;

#[path = "c3w_keep_both_ids.rs"]
mod c3w_keep_both_ids;

#[path = "c3w_rename_refs.rs"]
mod c3w_rename_refs;
#[path = "cm_rename_bytes.rs"]
mod cm_rename_bytes;
#[path = "rename_report_identities.rs"]
mod rename_report_identities;
#[path = "rename_would_change.rs"]
mod rename_would_change;

#[path = "cm_conformance_rename.rs"]
mod cm_conformance_rename;

#[path = "cm_merge_rules.rs"]
mod cm_merge_rules;

#[path = "cm_operation_cost.rs"]
mod cm_operation_cost;

#[path = "cm_conformance_names.rs"]
mod cm_conformance_names;

#[path = "cm_alias_merge.rs"]
mod cm_alias_merge;

#[path = "cm_favorites_rename.rs"]
mod cm_favorites_rename;
#[path = "cm_keys_publish.rs"]
mod cm_keys_publish;

#[path = "cm_conformance_published.rs"]
mod cm_conformance_published;

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
#[path = "journal_step_cost.rs"]
mod journal_step_cost;

#[path = "l8_03_duplicate_day_asset.rs"]
mod l8_03_duplicate_day_asset;

#[path = "l9_live_safety.rs"]
mod l9_live_safety;

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

mod merge_generated;
#[path = "merge_keeps_aliases.rs"]
mod merge_keeps_aliases;
mod merge_text_trace;

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

#[path = "page_op_walks.rs"]
mod page_op_walks;

#[path = "page_rename_identity.rs"]
mod page_rename_identity;
#[path = "page_rename_subfolder.rs"]
mod page_rename_subfolder;

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

#[path = "published_answers.rs"]
mod published_answers;

#[path = "quick_share.rs"]
mod quick_share;

#[path = "share_selector.rs"]
mod share_selector;

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

#[path = "org_link_spans_in_tags.rs"]
mod org_link_spans_in_tags;
