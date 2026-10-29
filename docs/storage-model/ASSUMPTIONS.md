# Storage model assumptions and mutation scope

This is the M0–M4 ledger for storage s3.5 with the SB1 s3.6 supplement. It records the boundary of the
existing evidence; it adds no formal claim. Model anchors are literal substrings
of `quint/storage-s36.qnt` (the shared s3.5 anchors remain), or, prefixed
`s37: `, of the s3.7 git-pull model `quint/storage-s37-gitpull.qnt`; the
meta-test also checks every model file against `quint/SHA256SUMS`. Code references use repository-relative `file#owner`;
test references name actual `#[test]` functions. The meta-test connects the
model bytes to the existing s3.5 transcription/replay hash and the separate
s3.6 Quint ITF endpoint fixture (G8), and checks all named
anchors, guards and tests. An existing test listed on an incomplete row covers
only the scope described below, not its pending environment fault.

Statuses: `guarded+tested` means the stated scope has a guard and existing fault
evidence; `guarded-untested` means a guard exists but the full assumption lacks
the required fault evidence; `unguarded` means at least the named scope lacks a
guard; `assumed-outside` means excluded from this storage proof. Guard existence
does not establish check ordering or root identity. Under SB2
(D-20261027-SB2), `guarded+tested` may describe completed desktop scope only
when the Platforms cell names that scope, its exact passing tests and runs,
and Android/iOS as mobile-pending; it does not assert those device obligations.
Martin decided this clarification and DESK-SCOPE in D-20261028-DSC; the desktop
scope below is settled, while the named tester acceptance remains pending.

| ID | Assumption | Model anchor | Guard (code) | Fault test | Platforms | Status |
|---|---|---|---|---|---|---|
| A-PATH-IDENTITY | Containment and original root binding are rechecked; publication metadata is checked after sync and retained debt checks entry/parent identity; sequential native races remain | `disk: int -> int` | `crates/tine-store/src/page_host/production_guard.rs#ProductionIo::check_path`<br>`crates/tine-store/src/page_host/production.rs#ProductionIo::page_sync`<br>`crates/tine-store/src/directory_durability.rs#DirectoryCreation::finish` | `crates/tine-store/src/page_host/production_guard_tests.rs#l8_01_actual_io_rechecks_after_admission_and_after_temp`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_save_schedules_refine_alpha_and_keep_durable_input`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_syscall_cross_product_including_trash_and_app_data`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_finding_c2_symlink_between_directory_plan_and_finish`<br>`crates/tine-store/src/directory_durability.rs#dircreate_rechecks_each_actual_parent_before_mkdir`<br>`crates/tine-store/src/directory_durability.rs#dircreate_sync_refusal_retains_unpaid_debt_until_safe_retry`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c1_c2_native_replay_matches_model_alpha_and_keeps_input`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c3_c4_native_and_model_do_not_publish_a_stale_recreation`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_parent_loss_after_publish_must_not_report_current_durable`<br>`crates/tine-store/src/transaction_environment_tests.rs#m3_transaction_apply_and_apply_move_c2_c3_escape_only`<br>`crates/tine-store/src/page_host/production_guard_tests.rs#windows_junction_at_mkdir_recheck_keeps_cold_draft_then_retries` | M2-2 and M3-1/M3-2 are active passing bounded regressions on identity-checking test storage. All five targets check binding/containment before mkdir and parent sync; retained debt checks entry and parent identities before/after sync. A detected replacement reports non-durable creation and retry re-plans the current chain. The retained host plan includes missing-component obligations even before any replacement mkdir records parent debt. Post-sync file metadata checks the publication stamp without payload I/O. Android directory identity stays vanish-only; weak/reused identities and swaps restored between sequential probes remain. An ancestor swapped/moved after the last relevant check can still redirect an empty mkdir outside; ordinary file publication retains its check/OS-call race. Linux bounded evidence preceded the hosted acceptance below. Directories are tested against the model, never proven. Windows C2 now has 16 required native junction phase samples plus root/parent mkdir-recheck refusal, physical cold-draft custody and restoration/retry witnesses; Windows hosted execution is recorded below.<br><br>Desktop scope for this round: local Windows NTFS, macOS APFS and Linux ext4 volumes, plus Windows UNC paths only to the extent witnessed below. Windows OneDrive Files On-Demand, macOS iCloud Drive cloud-placeholder folders and other network/FUSE mounts: **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**. W2: Windows Server 2025 Datacenter 10.0.26100, NTFS fixed drives, run [40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417). M2: macOS 26.6.2 (25G83), arm64, APFS local/journaled, run [40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537). Both at `8960039cc0544ab6adb59dbda23def3dfec8cfcc`. L3: Ubuntu 22.04.5 LTS, ext4 local volume, run [40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859) at `dbdb8cb46277d4a761d7404a79980b5f885806a9`. Runner temp, temp base and resolved app-data base are on the same ext4 volume. Mount options `nobarrier,data=writeback,journal_async_commit` supply no physical durability (power-loss) evidence; these are logical fault tests. L/E remain supporting historical evidence only. The 10 applicable Fault test functions in this row passed L3/M2; W2 passed the 7 applicable functions. Windows-only `windows_junction_at_mkdir_recheck_keeps_cold_draft_then_retries` ran only W2; Unix-only `l8_01_actual_io_rechecks_after_admission_and_after_temp`, `m2_finding_c2_symlink_between_directory_plan_and_finish`, `dircreate_rechecks_each_actual_parent_before_mkdir` and `dircreate_sync_refusal_retains_unpaid_debt_until_safe_retry` ran only L3/M2. Their Unix symlink harness is absent on Windows; W2's junction replay and mkdir-recheck/cold-custody/retry test cover the Windows guard. The Windows junction implementation is absent on Linux/macOS, which use the passing Unix witnesses. W2 native `m3_c1_c2_native_replay_matches_model_alpha_and_keeps_input` asserts 16 C1 + 16 C2 junction samples (print captured). All three F-M3S-1 tests passed L3/M2 at local temp and resolved app data, and W2 at local temp, resolved app data and loopback UNC. **Desktop promotion:** the named local-volume obligations are complete under SB2/DESK-SCOPE. UNC scope is limited to the three F-M3S-1 witnesses; no UNC F-DC-1 parent-substitution refusal/custody/retry witness executed, so that stronger UNC claim remains unsupported. Android/iOS: mobile-pending per SB2. Exact run/log keys and UNC limits: [SB1 receipts](#sb1-desktop-acceptance-2026-10-28). | guarded+tested |
| A-ROOT-BOUND | Application graph writes check the directory bound at open; Tine never creates the graph root or an ancestor | `disk: int -> int` | `crates/tine-store/src/root_binding.rs#RootBinding::check`<br>`crates/tine-store/src/directory_durability.rs#DirectoryCreation::new`<br>`crates/tine-store/src/directory_durability.rs#DirectoryCreation::finish` | `crates/tine-store/src/page_host/production_guard_tests.rs#l9_replaced_root_refuses_creating_save_and_keeps_input`<br>`crates/tine-store/src/directory_durability.rs#l9_directory_creation_never_recreates_a_vanished_bound`<br>`crates/tine-store/src/restore.rs#l9_restore_refuses_a_replaced_root_before_reserving_recovery`<br>`crates/tine-store/src/page_host/native_conformance_tests.rs#l9_native_and_model_refuse_the_same_detached_root_phases`<br>`crates/tine-store/src/page_host/binding_root_tests.rs#l9_input_typed_after_root_moves_reaches_durable_custody`<br>`crates/tine-store/src/root_binding.rs#l9_root_binding_holds_no_directory_descriptor`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_save_schedules_refine_alpha_and_keep_durable_input`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_syscall_cross_product_including_trash_and_app_data`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_finding_c1_replaced_root_between_directory_plan_and_finish`<br>`crates/tine-store/src/directory_durability.rs#dircreate_existing_directory_adds_no_binding_or_sync_work`<br>`crates/tine-store/src/page_host/binding_root_tests.rs#dircreate_failed_mkdir_discard_close_releases_graph_handles`<br>`crates/tine-store/src/page_host/binding_root_tests.rs#dircreate_new_folder_save_needs_no_root_read_permission`<br>`crates/tine-store/src/directory_durability.rs#dircreate_checks_original_open_binding_even_if_replaced_before_planning`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c1_c2_native_replay_matches_model_alpha_and_keeps_input`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_native_m2_1_replaced_root_between_plan_and_finish`<br>`crates/tine-store/src/page_host/production_guard_tests.rs#windows_junction_at_mkdir_recheck_keeps_cold_draft_then_retries` | M2-1 is active on identity-checking targets with unchanged outcome assertions. Every mkdir reuses the original open-time RootBinding; Linux/macOS/iOS keep existing stat/statfs policy, Windows FILE_ID_INFO, Android vanish-only (same-path replacement remains undetected). SMB/FUSE relaxed identities and inode reuse/unavailable birth time retain their existing limits. No directory handles or new read permission requirement. The host retains failed plans with recorded parent debt or missing components across Discard/page Close and retries until successful completion or adapter close. Re-planning transfers custody to the current chain's parent debt and missing-component obligations. Honest contained-parent replacement can retry after re-planning. Initial open-time capture, final check-to-create, parent rename/check-to-sync, and ordinary file-publication windows remain; no native no-escape proof. ModelFs tests alpha/escapes/draft custody; M3 bounded native phase samples (Linux executed) check α, escapes and durable input retention, with Android same-path replacement excluded from native replay; hosted local-volume execution is recorded below. Developer export remains outside; Git metadata uses the SB1 guard below and remains outside full model-oracle refinement. Windows root and parent junction mkdir-recheck/cold-recovery/retry witnesses are wired into the store suite; hosted execution is recorded below.<br><br>Desktop scope for this round: local Windows NTFS, macOS APFS and Linux ext4 volumes, plus Windows UNC paths only to the extent witnessed below. Windows OneDrive Files On-Demand, macOS iCloud Drive cloud-placeholder folders and other network/FUSE mounts: **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**. W2: Windows Server 2025 Datacenter 10.0.26100, NTFS fixed drives, run [40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417). M2: macOS 26.6.2 (25G83), arm64, APFS local/journaled, run [40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537). Both at `8960039cc0544ab6adb59dbda23def3dfec8cfcc`. L3: Ubuntu 22.04.5 LTS, ext4 local volume, run [40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859) at `dbdb8cb46277d4a761d7404a79980b5f885806a9`. Runner temp, temp base and resolved app-data base are on the same ext4 volume. Mount options `nobarrier,data=writeback,journal_async_commit` supply no physical durability (power-loss) evidence; these are logical fault tests. L/E remain supporting historical evidence only. All 15 applicable Fault test functions in this row passed L3; W2/M2 passed all 13 applicable functions each. `l9_root_binding_holds_no_directory_descriptor` and `dircreate_failed_mkdir_discard_close_releases_graph_handles` are Linux-only; `dircreate_new_folder_save_needs_no_root_read_permission` is Unix-only; `windows_junction_at_mkdir_recheck_keeps_cold_draft_then_retries` is Windows-only. The two Linux-only tests census `/proc/self/fd`, an instrument absent on Windows/macOS, rather than an untested guard there. The Unix permission-bit fixture has no Windows counterpart; native Windows binding/junction tests cover its platform policy. The Windows junction implementation is absent on Linux/macOS. All other named functions passed L3/W2/M2, including native root/parent replay and `m3_native_m2_1_replaced_root_between_plan_and_finish`. Unix F-DC-2 `dircreate_swap_during_save_keeps_input_in_durable_device_draft` and R2 `m3_partial_replan_refusal_retains_root_sync_until_restoration` passed L3/M2; their Unix symlink fixtures and directory-sync implementation are absent on Windows, where W2 passed the junction cold-custody/retry counterpart without claiming a directory flush. Native corpus and metadata-only cost guard passed L3/W2/M2. **Desktop promotion:** the named local-volume obligations are complete under SB2/DESK-SCOPE. UNC scope is limited to F-M3S-1; the F-DC-1 root/parent substitution, refusal/custody and retry witnesses did not execute there, so that stronger UNC claim remains unsupported. Android/iOS: mobile-pending per SB2, including Android FUSE/same-path limits. Exact run/log keys and UNC limits: [SB1 receipts](#sb1-desktop-acceptance-2026-10-28). | guarded+tested |
| A-DRAFT-NAMESPACE | After a root path change, pre-crash drafts are restored or surfaced for explicit adoption | `action launch` | `src-tauri/src/root_identity.rs#find_orphans`<br>`src-tauri/src/root_identity.rs#adopt_namespace` | `src-tauri/src/root_identity.rs#relocation_is_idempotent_and_preserves_sealed_bytes`<br>`src-tauri/src/root_identity_m4_environment_tests.rs#m4_c5_native_moved_graph_surfaces_every_old_namespace_vehicle`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c5_model_relaunch_at_new_path_surfaces_then_recovers_every_draft`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c5_native_moved_root_keeps_physical_orphans_and_recovers_after_adoption` | C5 composes app census/adoption with cold adapter recovery of existing, absent and changed-base records. Desktop relocation surfaces an orphan before explicit adoption; it never merges automatically. Known/unknown old roots and occupied targets are tested. Existing formats and validated iOS relocation remain; iOS device rebase remains unverified. Hosted desktop app executions now include the C5 census/adoption family; executed results are recorded below. See `.github/workflows/ci.yml` for the desktop acceptance steps.<br><br>Desktop scope for this round: local Windows NTFS, macOS APFS and Linux ext4 volumes, plus Windows UNC paths only to the extent witnessed below. Windows OneDrive Files On-Demand, macOS iCloud Drive cloud-placeholder folders and other network/FUSE mounts: **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**. W2: Windows Server 2025 Datacenter 10.0.26100, NTFS fixed drives, run [40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417). M2: macOS 26.6.2 (25G83), arm64, APFS local/journaled, run [40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537). Both at `8960039cc0544ab6adb59dbda23def3dfec8cfcc`. L3: Ubuntu 22.04.5 LTS, ext4 local volume, run [40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859) at `dbdb8cb46277d4a761d7404a79980b5f885806a9`. Runner temp, temp base and resolved app-data base are on the same ext4 volume. Mount options `nobarrier,data=writeback,journal_async_commit` supply no physical durability (power-loss) evidence; these are logical fault tests. L/E remain supporting historical evidence only. All four Fault test functions in this row executed/passed L3/W2/M2: `relocation_is_idempotent_and_preserves_sealed_bytes`, `m4_c5_native_moved_graph_surfaces_every_old_namespace_vehicle`, `m4_c5_model_relaunch_at_new_path_surfaces_then_recovers_every_draft`, and `m4_c5_native_moved_root_keeps_physical_orphans_and_recovers_after_adoption`. Named desktop census/adoption and cold-adapter witnesses are complete; no zero-test filter is credited. **Desktop promotion:** the named local-volume obligations are complete under SB2/DESK-SCOPE. No UNC census/adoption acceptance is inferred from the separate F-M3S-1 harness. Android/iOS: mobile-pending per SB2; iOS update/container rebase requires a demonstrated real container move. Exact run/log keys and UNC limits: [SB1 receipts](#sb1-desktop-acceptance-2026-10-28). | guarded+tested |
| A-ONE-HOST | One host owns a graph and its drafts, while owner arbitration is available | `alive: bool` | `src-tauri/src/owner/lease.rs#claim`<br>`src-tauri/src/owner_protection.rs#retry_with`<br>`src-tauri/src/state.rs#GraphRegistry::draft_namespace_held` | `src-tauri/src/root_identity.rs#old_graph_id_uses_the_same_owner_lock_as_the_host`<br>`src-tauri/src/host_retirement.rs#l9_retiring_source_refuses_draft_recovery`<br>`src-tauri/src/host_retirement.rs#l9_recovery_and_relocation_exclude_both_bound_and_retiring_namespaces`<br>`src-tauri/src/owner/namespace.rs#windows_and_unix_enable_the_shared_owner_protocol`<br>`src-tauri/src/owner/owner_tests.rs#long_namespace_retains_the_owner_lock_without_a_command_endpoint`<br>`src-tauri/src/owner_protection.rs#retries_attach_once_and_cancelled_bindings_never_take_a_lease`<br>`src-tauri/src/root_identity.rs#recovery_without_current_lease_refuses_before_adopting_any_input` | Linux/macOS/Windows account locks; iOS/Android per-app sandbox cross-process exclusion. All five retain registry/retirement draft exclusion, except the saved-stop recovery target. D-20261027-OF: failed claims keep editing with a persistent fixable warning and retry every 3 s after the preceding attempt via custody-checked handover (up to 60 s). Two hosts can overlap from failure until successful acquisition, including handover; persistent failure/refusal makes this interval unbounded. Recovery refuses without the current desktop lease. Long account homes no longer defeat the lock; endpoint limits are separate. Earlier Linux native repair/handover passed; current hosted desktop test execution is recorded below. The command's shared checked helper directly refuses without the current desktop lease before any adoption; an otherwise-adoptable physical census remains intact. Hosted recovery/retirement execution is recorded below. See `.github/workflows/ci.yml` for the desktop acceptance steps. The prior Linux native repair/handover journey is `scripts/e2e-owner-protection.mjs`.<br><br>Desktop scope for this round: local Windows NTFS, macOS APFS and Linux ext4 volumes, plus Windows UNC paths only to the extent witnessed below. Windows OneDrive Files On-Demand, macOS iCloud Drive cloud-placeholder folders and other network/FUSE mounts: **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**. W2: Windows Server 2025 Datacenter 10.0.26100, NTFS fixed drives, run [40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417). M2: macOS 26.6.2 (25G83), arm64, APFS local/journaled, run [40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537). Both at `8960039cc0544ab6adb59dbda23def3dfec8cfcc`. L3: Ubuntu 22.04.5 LTS, ext4 local volume, run [40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859) at `dbdb8cb46277d4a761d7404a79980b5f885806a9`. Runner temp, temp base and resolved app-data base are on the same ext4 volume. Mount options `nobarrier,data=writeback,journal_async_commit` supply no physical durability (power-loss) evidence; these are logical fault tests. L/E remain supporting historical evidence only. All seven Fault test functions in this row passed L3/M2. W2 passed all six applicable functions: `old_graph_id_uses_the_same_owner_lock_as_the_host`, `l9_retiring_source_refuses_draft_recovery`, `l9_recovery_and_relocation_exclude_both_bound_and_retiring_namespaces`, `windows_and_unix_enable_the_shared_owner_protocol`, `retries_attach_once_and_cancelled_bindings_never_take_a_lease`, and `recovery_without_current_lease_refuses_before_adopting_any_input`; Unix-only `long_namespace_retains_the_owner_lock_without_a_command_endpoint` did not execute W2: its overlong Unix-domain-socket endpoint guard is absent on Windows, which uses named pipes, so this cfg exclusion is not an unmet Windows obligation. Earlier Linux real-app gate E passed warning/edit/repair/handover retry 3/3 at `4b1e1bf6d`; L is no-app and L/E have no logged OS/filesystem identity. L3 supplies the current identified Linux app-crate tests; E retains the previously accepted real-app journey. **Desktop promotion:** the named local-volume obligations are complete under SB2/DESK-SCOPE. The UNC volume harness does not test owner arbitration/recovery exclusion there. Failed arbitration still admits potentially unbounded overlap until successful acquisition. Android/iOS sandbox/exclusion: mobile-pending per SB2. Exact run/log keys and UNC limits: [SB1 receipts](#sb1-desktop-acceptance-2026-10-28). | guarded+tested |
| A-DIRSYNC | Directory sync settles names under the platform identity policy; publication and retained-debt checks reject detected parent loss/replacement | `action dirSync` | `crates/tine-store/src/directory_durability.rs#DirectoryCreation::finish`<br>`crates/tine-store/src/page_host/production.rs#ProductionIo::page_sync` | `crates/tine-store/tests/directory_durability_guard.rs#directory_sync_has_one_owner_and_no_discarded_result`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_parent_loss_after_publish_must_not_report_current_durable`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_c4_retained_directory_sync_must_not_acknowledge_a_new_inode`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_c4_model_retained_directory_sync_must_not_acknowledge_a_new_inode`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m3_c6_classified_errors_refine_save_fail_and_failed_dirsync`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c6_native_withdraw_witnesses_durable_chain_and_same_volume_move`<br>`crates/tine-store/src/page_host/production_guard_tests.rs#windows_junction_at_mkdir_recheck_keeps_cold_draft_then_retries` | M3-1/M3-2 active and passing on Linux; tested against the model. NotFound fails sync; changed publication stays Uncertain. Stale directory debt reports non-durable creation, then retry syncs the current chain. Missing components retain host custody through a refusal before the first replacement mkdir; restoring an earlier ancestor still requires its parent sync. Android directory checks stay vanish-only (native M3-2 replacement excluded); Windows performs identity checks without directory flush. Weak/reused identity, sequential substitution/restoration and untested desktop/device volumes remain open. M4 verifies withdraw's existing bounded durable chain on fresh/existing conflict trash and ModelFs power outcomes under the same-volume single-move recipe; no native power-loss claim. Windows junction phase replay includes the directory-sync boundary; root/parent mkdir refusal and cold custody/retry are implemented, with hosted execution recorded below.<br><br>Desktop scope for this round: local Windows NTFS, macOS APFS and Linux ext4 volumes, plus Windows UNC paths only to the extent witnessed below. Windows OneDrive Files On-Demand, macOS iCloud Drive cloud-placeholder folders and other network/FUSE mounts: **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**. W2: Windows Server 2025 Datacenter 10.0.26100, NTFS fixed drives, run [40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417). M2: macOS 26.6.2 (25G83), arm64, APFS local/journaled, run [40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537). Both at `8960039cc0544ab6adb59dbda23def3dfec8cfcc`. L3: Ubuntu 22.04.5 LTS, ext4 local volume, run [40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859) at `dbdb8cb46277d4a761d7404a79980b5f885806a9`. Runner temp, temp base and resolved app-data base are on the same ext4 volume. Mount options `nobarrier,data=writeback,journal_async_commit` supply no physical durability (power-loss) evidence; these are logical fault tests. L/E remain supporting historical evidence only. All six portable Fault test functions in this row executed/passed L3/W2/M2; the seventh, `windows_junction_at_mkdir_recheck_keeps_cold_draft_then_retries`, passed only W2: its native junction implementation is absent on Linux/macOS. Unix R2 `m3_partial_replan_refusal_retains_root_sync_until_restoration` passed L3/M2. Its Unix symlink/real-directory-sync path is absent on Windows, which uses identity checks without directory flush; W2's native junction replay and cold-custody/retry test cover that policy. W2 native phase replay asserts 16 C2 junction samples including DirectorySync (print captured). All three F-M3S-1 tests passed L3/M2 at local temp and resolved app data, and W2 at local temp, resolved app data and loopback UNC. **Desktop promotion:** the named local-volume obligations are complete under SB2/DESK-SCOPE. UNC scope is limited to F-M3S-1; substituted-parent refusal/custody/retry did not execute there, so that stronger UNC claim remains unsupported. Windows identity checks do not flush directories, and no physical power-loss proof is inferred. Android/iOS backing-volume checks: mobile-pending per SB2. Exact run/log keys and UNC limits: [SB1 receipts](#sb1-desktop-acceptance-2026-10-28). | guarded+tested |
| A-ATOMIC-READ | Reads return a whole version; arbitrary torn reads remain outside the model | `disk: int -> int` | `crates/tine-store/src/page_host/save.rs#Host::advance_save_locked` | `crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c7_model_partial_external_reads_conflict_without_publishing`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c7_native_in_place_writer_during_read_conflicts_and_keeps_input` | C7 tests empty/interior/almost-complete prefixes at Open and save Check; a barrier-controlled native thread truncates/writes after reader open/stat. Samples conflict, retain input durably and never publish. No atomic-read guard, arbitrary overlapping writers or final-check/rename race guarantee is inferred. Native Linux executed; other hosted targets pending. | assumed-outside |
| A-DRAFT-DURABLE | App-data drafts require a strong backing store and persistent namespace; detected volatile locations are disclosed without refusing graph open | `drafts: int -> Draft` | `crates/tine-store/src/page_host/production.rs#ProductionIo::draft_sync`<br>`src-tauri/src/data_home.rs#classify_at_launch`<br>`crates/tine-store/src/directory_durability/windows.rs#sync_directory` | `crates/tine-store/src/page_host/native_conformance_tests.rs#native_committed_scenarios_in_four_profiles`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c6_errno_classes_keep_sticky_failure_and_admitted_input`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c6_draft_failed_matches_i8_host_apply_row`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c6_errno_notice_stays_sticky_until_publication_on_both_backends`<br>`src-tauri/src/data_home.rs#sb1_data_home_classifies_memory_and_temporary_storage_without_refusing`<br>`src-tauri/src/data_home.rs#sb1_data_home_real_tmpfs_probe`<br>`src-tauri/src/data_home.rs#sb1_data_home_platform_policy_names_all_shipped_targets`<br>`crates/tine-store/src/page_host/storage_acceptance_tests.rs#uncertain_save_recovers_physical_draft_on_cold_reopen`<br>`crates/tine-store/src/directory_durability/windows.rs#windows_directory_flush_succeeds_on_hosted_ntfs`<br>`crates/tine-store/src/directory_durability/windows.rs#windows_directory_flush_failures_preserve_graph_and_private_results`<br>`crates/tine-store/src/directory_durability/windows.rs#windows_flush_failure_after_open_is_counted_and_preserves_wrapper_results` | C6 StorageFull/EIO-class Other/PermissionDenied/ReadOnlyFilesystem retains admitted text and sticky errors through reattach/retry. Linux executed; hosted C6 backing-store execution is recorded below; device acceptance remains pending. Linux/Android classify tmpfs/ramfs by statfs magic; all five targets flag temp/runtime paths; Apple detects named memory filesystems and Windows RAM drives. Probe errors and RAM images presenting as ordinary disks remain unverified. Windows stated limit (D-20261028-WDD): no Win32-documented directory flush establishes draft-folder creation/removal persistence or the ordinary-rename fallback. Tine attempts a best-effort app-data folder flush with CreateFileW + FlushFileBuffers; failures are counted and ignored, graph folders untouched. This hardening supplies no unconditional crash/power-loss guarantee or proof of unprivileged operation; NTFS journaling alone is insufficient. The row records desktop acceptance under this decided limit, not proof of Windows namespace persistence; see the platform note below. Hosted steps point C6 and cold-recovery witnesses at the resolved Tine app-data base, using disposable subdirectories. See `.github/workflows/ci.yml` for the desktop acceptance steps.<br><br>Desktop scope for this round: local Windows NTFS, macOS APFS and Linux ext4 volumes, plus Windows UNC paths only to the extent witnessed below. Windows OneDrive Files On-Demand, macOS iCloud Drive cloud-placeholder folders and other network/FUSE mounts: **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**. W2: Windows Server 2025 Datacenter 10.0.26100, NTFS fixed drives, run [40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417). M2: macOS 26.6.2 (25G83), arm64, APFS local/journaled, run [40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537). Both at `8960039cc0544ab6adb59dbda23def3dfec8cfcc`. L3: Ubuntu 22.04.5 LTS, ext4 local volume, run [40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859) at `dbdb8cb46277d4a761d7404a79980b5f885806a9`. Runner temp, temp base and resolved app-data base are on the same ext4 volume. Mount options `nobarrier,data=writeback,journal_async_commit` supply no physical durability (power-loss) evidence; these are logical fault tests. L/E remain supporting historical evidence only. Of the eight pre-existing Fault test functions, L3 passed all eight, including separately selected `uncertain_save_recovers_physical_draft_on_cold_reopen` at runner temp and the resolved app-data base. W2/M2 passed all seven applicable functions; Linux-only `sb1_data_home_real_tmpfs_probe` did not run there: Linux/Android statfs-magic detection is absent on Windows/macOS, which use RAM-drive/named-filesystem detection covered by the portable classifier/policy tests. Its passing Linux wrapper allows a skip when no usable tmpfs exists; no extra tmpfs branch execution is inferred. Cold recovery passed W2 three times (local temp, resolved app-data base, loopback UNC) and L3/M2 twice each (local temp, resolved app-data base), separately selected despite broad-suite ignore. C6 failure/admission/reattach/retry also passed at the resolved app-data bases using disposable subdirectories. WF: Windows run [40624305046](https://github.com/martinkoutecky/tine/actions/runs/40624305046) at `b31f594dd93f26ac139a8eb34905ebd49f78a75c` executed/passed all three added Windows-only flush tests: `windows_directory_flush_succeeds_on_hosted_ntfs`, `windows_directory_flush_failures_preserve_graph_and_private_results`, and `windows_flush_failure_after_open_is_counted_and_preserves_wrapper_results`. They cover API success, missing/exclusively opened directories, post-open flush failure, diagnostic increments and preserved wrapper results; graph wrappers do not flush. The Windows implementation is absent on Linux/macOS; no pass there is inferred. WF has no independent filesystem-type probe and is credited only for this Windows API/error-policy evidence. Its app-level `namespace_sync_preserves_results_and_uses_the_private_windows_boundary` test did not execute and is not credited. **Desktop promotion:** the named local-volume logical-fault/process-recovery obligations are complete under SB2/DESK-SCOPE and the stated Windows limit D-20261028-WDD. L3/M2 supply executed Linux/macOS evidence; W2 supplies NTFS/app-data recovery and WF the added flush tests. Successful NTFS/UNC process recovery and elevated API acceptance do not establish crash-persistent namespaces, unprivileged access or physical power-loss durability. UNC acceptance remains limited to the executed F-M3S-1 cold-recovery witness; no remote-server or UNC flush guarantee. Android/iOS app-data/fault-lab acceptance: mobile-pending per SB2. Exact run/log keys and UNC limits: [SB1 receipts](#sb1-desktop-acceptance-2026-10-28). | guarded+tested |
| A-CASE-DISTINCT | Distinct page keys name distinct files, subject to physical spelling aliases | `pure val PAGES` | `crates/tine-store/src/no_replace.rs#move_file_noreplace`<br>`crates/tine-store/src/transaction/preflight.rs#PagePlan::page_create_checks`<br>`crates/tine-store/src/transaction/move_file.rs#PagePlan::source_spelling_alias` | `crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c8_model_case_alias_is_seen_by_admission_and_never_overwritten`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#sb1_c8_model_unicode_case_aliases_refuse_collision`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#sb1_c8_model_normalization_aliases_refuse_collision`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#sb1_c8_model_full_fold_aliases_refuse_collision`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#sb1_c8_model_turkish_i_uses_default_non_turkic_fold`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#sb1_c8_native_unicode_admission_and_noreplace_follow_volume`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#sb1_c8_native_unicode_self_alias_rename_keeps_bytes`<br>`crates/tine-store/src/transaction/move_file.rs#sb1_unicode_self_aliases_allow_spelling_moves` | SB1 extends C8 ModelFs Case to Unicode 9 default full folding plus canonical normalization, including Kůň/KŮŇ/kůň, NFC/NFD, ß/SS, sigma and dotted i; default folding keeps dotless i distinct from I. Production delegates occupancy and no-replace to the actual directory resolver on every platform, including unknown Android volumes; no global fold rejects sensitive-volume names. Self-alias renames require filesystem identity and an unlisted destination; listings preserve exact NFC/NFD entries. Linux native probes find the seven Unicode pairs distinct and exercise independent load/save controls; folding-volume self-renames explicitly report NATIVE_MODELFS_ONLY there. Hosted Windows/macOS Unicode execution and filesystem identity are recorded below. Prior ASCII native acceptance: NTFS CI 40549012820 and default APFS CI 40562543184, 2026-10-27. Android shared-storage NFC/NFD device evidence is pending; on the Android emulator's shared storage (MediaProvider FUSE, API 33 and 35, CI runs 40752778804 and 40754122268) a case-only rename(2) succeeds but the directory keeps the old spelling, so the page host's self-alias move refuses (SpellingIncomplete) and writes nothing; fallback check-to-rename races remain as documented in no_replace.rs.<br><br>Desktop scope for this round: local Windows NTFS, macOS APFS and Linux ext4 volumes, plus Windows UNC paths only to the extent witnessed below. Windows OneDrive Files On-Demand, macOS iCloud Drive cloud-placeholder folders and other network/FUSE mounts: **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**. W2: Windows Server 2025 Datacenter 10.0.26100, NTFS fixed drives, run [40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417). M2: macOS 26.6.2 (25G83), arm64, APFS local/journaled, run [40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537). Both at `8960039cc0544ab6adb59dbda23def3dfec8cfcc`. L3: Ubuntu 22.04.5 LTS, ext4 local volume, run [40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859) at `dbdb8cb46277d4a761d7404a79980b5f885806a9`. Runner temp, temp base and resolved app-data base are on the same ext4 volume. Mount options `nobarrier,data=writeback,journal_async_commit` supply no physical durability (power-loss) evidence; these are logical fault tests. L/E remain supporting historical evidence only. All eight Fault test functions in this row executed/passed L3/W2/M2. `sb1_c8_native_unicode_admission_and_noreplace_follow_volume` requires Unicode case collisions on W2/M2, normalization/full-fold collisions on M2, and distinct ß/SS on W2. `sb1_c8_native_unicode_self_alias_rename_keeps_bytes` requires nonzero native alias execution on W2/M2; Linux ext4 permits zero aliases, so its ok result is a case-sensitive-volume control, not folding-volume acceptance or a missing ext4 obligation; the guard delegates to the native resolver. `sb1_unicode_self_aliases_allow_spelling_moves` and ASCII `m4_c8_native_case_insensitive_volume_admission_refuses_collision` passed W2/M2 (ASCII twice M2). The three hosted runs identify the actual ext4/NTFS/APFS volumes; exact alias count prints remain captured. **Desktop promotion:** the named local-volume obligations are complete under SB2/DESK-SCOPE. No UNC Unicode acceptance is inferred from F-M3S-1. Android/iOS: mobile-pending per SB2, especially Android shared-storage NFC/NFD. Fallback check-to-rename races remain. Exact run/log keys and UNC limits: [SB1 receipts](#sb1-desktop-acceptance-2026-10-28). | guarded+tested |
| A-ATOMIC-REPLACE | Publication is atomic old-or-new under the platform recipe | `action rename` | `crates/tine-store/src/atomic_file.rs#rename_replace` | `crates/tine-store/tests/internal_crash_transactions.rs#transaction_kill_reopen_preserves_each_old_or_new_file`<br>`crates/tine-store/tests/i16_platform_arms.rs#host_publication_witnesses_cover_exactly_the_five_shipped_targets` | All five recipes pinned; weak filesystems retain weaker durability | guarded+tested |
| A-DELETE-ONE-MOVE | Delete moves once into same-volume trash without replacing a target | `action rename` | `crates/tine-store/src/no_replace.rs#move_file_noreplace` | `crates/tine-store/src/page_host/native_conformance_tests.rs#native_committed_scenarios_in_four_profiles` | All five; unsupported rename-noreplace fallback has accepted creator race; errno-class test pending | guarded+tested |
| A-TRASH-COUPLED | A durable page's bytes survive deletion or withdrawal in its page or trash payload | `trashStable: int -> Set[int]` | `crates/tine-store/src/page_host/production.rs#ProductionIo::trash_sync`<br>`crates/tine-store/src/model.rs#Graph::withdraw_file_to_conflict_if` | `crates/tine-store/src/page_host/conformance_tests.rs#all_s3_scenarios_in_four_profiles`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c6_withdraw_model_power_cuts_keep_page_or_conflict_payload`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c6_native_withdraw_witnesses_durable_chain_and_same_volume_move` | Existing page-host oracle power cuts plus M4's six withdrawal primitive power outcomes: durable chain and same-volume atomic move retain old source or conflict payload (surplus copy allowed). Native verifies chain sync requests and exact displaced bytes, not physical power loss. Full transaction/withdraw refinement, weak filesystems and external unflushed payload data remain outside this added scope. | guarded+tested |
| A-NAME-SLOT | Operation and ordinary creating-save admission check another claimant of the same logical name; blocked input is retained | `val claimedCreate =` | `crates/tine-store/src/page_host/production_guard.rs#ProductionIo::creating_owner` | `crates/tine-store/src/page_host/s36_creation_tests.rs#s36_plain_creation_replays_binding_admission_and_retains_input`<br>`crates/tine-store/src/page_host/s36_creation_tests.rs#s36_unrelated_claim_does_not_block_plain_creation`<br>`crates/tine-store/src/page_host/production_guard_tests.rs#l8_02_raw_creation_rules_survive_failure_and_new_input_revokes_them` | Shared five-target path; s3.6 disabled creating flush, physical title/twin/unreadable arrival, binding submit, refusal, physical durable draft and departure/retry tested on Linux. No permanent uniqueness or native check-to-call guarantee. Other target runtime acceptance remains separate. | guarded+tested |
| A-EXT-WHOLE | Environment writes replace whole files | `action extWriteD` | — | — | All five; in-place writes excluded, see A-ATOMIC-READ | assumed-outside |
| A-PROCESS-CUT | Process/window may die between syscalls | `action crash` | `crates/tine-store/src/atomic_file.rs#PreparedWrite::with_hooks` | `crates/tine-store/tests/internal_crash_transactions.rs#transaction_kill_reopen_preserves_each_old_or_new_file`<br>`src-tauri/src/owner/git_mobile_tests.rs#a_pull_the_os_ends_resumes_to_the_fetched_tip` | Page-host phase cuts and selected subprocess journeys, including process kills across a git pull's apply (A-GIT-PULL); not native power-loss refinement (G7) | guarded+tested |
| A-REFS-GIVEN | Referrer discovery is supplied correctly | `action opRename` | — | — | All five; index/referrer correctness outside storage proof | assumed-outside |
| A-OPAQUE-TEXT | Texts abstract bytes as small integers | `Texts are small integers` | — | — | All five; serialization/round-trip obligations SPEC-s2 §4.1–4.6 outside model | assumed-outside |
| A-PROGRESS | Under weakly fair scheduling, advancing time, finite input/interference, released locks and successful draft I/O, the default driver publishes saveable edits or drafts blocked input | `action draftSync` | `crates/tine-store/src/page_host/driver.rs#Shared::submit`<br>`crates/tine-store/src/page_host/progress.rs#Progress::poll` | `crates/tine-store/src/page_host/s36_progress_tests.rs#g3_default_progress_replays_quint_endpoints_through_driver`<br>`crates/tine-store/src/page_host/driver_tests.rs#every_admission_wakes_a_sleeping_driver` | Portable driver/ModelFs replay: 5 journeys x 4 profiles on Linux; production submit urgency enabled throughout. A finite delivery barrier orders conflict interference; save faults precede admission. 5s wall and 10,000ms virtual advance per checkpoint. Covers publish, conflict draftSync, failed-save draftSync, draft refresh, finite three-page service (not simultaneous due-page fairness). OS fairness and unbounded temporal liveness are assumptions, not proved; other target runtime acceptance remains separate. | guarded+tested |
| A-MUTATION-CONTAINMENT | Application Git metadata mutations check bound graph/repository directories and target containment; developer export and fault-injection helpers remain exceptions | `disk: int -> int` | `src-tauri/src/owner/git_run.rs#GitMutationGuard::check_path`<br>`src-tauri/src/owner/git_run.rs#GitMutationGuard::check_git`<br>`crates/tine-store/src/root_binding.rs#BoundDirectory::check_path` | `src-tauri/src/owner/git_autocommit_tests.rs#sb1_autocommit_refuses_root_replacement_before_index_lock`<br>`src-tauri/src/owner/git_autocommit_tests.rs#sb1_index_lock_refuses_a_redirected_index`<br>`src-tauri/src/owner/git_autocommit_tests.rs#sb1_index_cleanup_preserves_files_after_metadata_root_replacement`<br>`src-tauri/src/owner/git_autocommit_tests.rs#sb1_repack_refuses_redirected_pack_directory`<br>`src-tauri/src/owner/git_autocommit_tests.rs#sb1_libgit_refuses_redirected_object_fanout`<br>`src-tauri/src/owner/git_autocommit_tests.rs#sb1_git_directory_alias_is_refused_before_any_metadata_write`<br>`src-tauri/src/owner/owner_tests/git_run_safety_tests.rs#sb1_run_refuses_the_root_replaced_since_store_open` | Git uses libgit2 on all five targets (Android and iOS since ADR 0078 slice 3, vendored, fsync on; Android turns libgit2's repository-owner check off because MediaProvider FUSE reports another owner for every shared-storage file). Linux fault evidence; the named tests also executed and passed hosted Windows and macOS runs W3/M3 (SB1-GITSYNC, Git sync section), Unix-only witnesses on macOS only. Android: `src-tauri/src/owner/git_mobile_tests.rs` runs commit, pull, push, repack and 13 SIGKILLs mid-commit/push on the emulator's shared storage (FUSE, API 33 and 35, `android-git-storage` in `ci.yml`); a case-only pull refuses there (MediaProvider keeps the old spelling, see A-CASE-DISTINCT). iOS: compile only. Android/iOS device acceptance: checked by testers after beta.1 (SB2). Shared containment/root identity has explicit Linux, macOS, Windows, Android and iOS policy; Android is vanish-only, weak/reused identities and sequential check-to-call/internal libgit2 race windows remain. Git-selected parent repository and linked-worktree metadata scopes remain supported. Developer export CLI and test-injection helpers stay unguarded; no universal containment or Git model refinement claim. Redirected pack/object directories and a .git directory alias now have unprivileged Windows junction fixtures in the hosted owner filter; they and the root-replacement witnesses passed W3 and M3 (SB1-GITSYNC), with the redirected index-file symlink witness Unix-only (M3 only). Fetch: `file_fetch_updates_only_tracking_ref_and_preserves_dirty_worktree_and_index` passed W3/M3. | unguarded |
| A-GIT-PULL | A pull writes the graph only through the audited page host and store transactions, over `H`'s bytes; the branch reaches `U` only after every changed path and the index hold `U` durably; a process cut anywhere resumes to `U` | `s37: val gitSafety =` | `src-tauri/src/owner/git_pull.rs#pre_check`<br>`src-tauri/src/owner/git_pull.rs#apply_pull`<br>`src-tauri/src/owner/git_pull.rs#barrier`<br>`src-tauri/src/owner/git_pull.rs#defers_autocommit`<br>`src-tauri/src/owner/git_autocommit.rs#without_tine_own`<br>`src-tauri/src/owner/git_pull.rs#own_entries_at`<br>`src-tauri/src/owner/git_run.rs#IndexLock::install`<br>`src-tauri/src/owner/git_run.rs#Repo::advance` | `src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_pull_lands_a_fast_forward`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_every_crash_point_keeps_head_and_index_consistent`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_crash_between_index_and_ref_resumes`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_barrier_syncs_paths_already_at_u`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_page_changed_after_the_pre_pull_autocommit_holds_then_diverges`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_resume_test_that_cannot_scan_defers_the_autocommit`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_resume_test_that_cannot_hash_a_file_defers_the_autocommit`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_outside_edit_to_a_pulled_file_during_the_apply_is_kept`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_page_rename_lands_and_resumes`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_case_only_rename_keeps_both_sides`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#r2_unicode_case_only_rename_lands_on_sensitive_volume`<br>`src-tauri/src/owner/git_mobile_tests.rs#a_pull_the_os_ends_resumes_to_the_fetched_tip`<br>`src-tauri/src/owner/git_mobile_tests.rs#a_pulled_case_only_rename_lands_on_the_graph_volume`<br>`src-tauri/src/owner/git_mobile_tests.rs#a_pulled_deletion_does_not_make_the_next_pull_diverge`<br>`src-tauri/src/owner/git_mobile_tests.rs#another_devices_trash_copy_does_not_stop_the_pull`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_pull_skips_tine_own_paths_without_refusing`<br>`src-tauri/src/owner/owner_tests/git_pull_tests.rs#q_git_own_entries_resume_across_index_and_ref`<br>`src-tauri/src/owner/git_autocommit_tests.rs#committed_own_files_are_untracked_in_their_own_commit_with_the_next_change` | A git pull (ADR 0078 §4 rev 2) changes the graph only through one page-host operation per rename plus one for the other pages (`HostOp`, `Input::Refuse`, planned from `H`'s bytes) and one revision-guarded store transaction per other file, so its file writes inherit A-ATOMIC-REPLACE, A-PATH-IDENTITY, A-ROOT-BOUND, A-CASE-DISTINCT, A-DELETE-ONE-MOVE and A-TRASH-COUPLED (pulled deletions keep their bytes in the graph trash). This row adds the s3.7 order: refuse by name before any write, then pages, other files, the barrier (fsync every changed path, those already at `U` included, and its folder), the index install, then the ref compare-and-swap (P1, P1b, P2); every autocommit defers while the tree is a half-applied pull (P3, P4), and also when the status scan behind that test, or its read of one differing file, fails (REG-SB1-GIT-RESUME-STATUS-ERROR, REG-SB1-GIT-RESUME-HASH-ERROR). s3.7 is model-checked in Quint (gs-model receipt); the code is tested against its named qGit scenarios, not refined against the model. `a_pull_the_os_ends_resumes_to_the_fetched_tip` kills the process across a 150-page pull with a rename, a non-page file and a deletion (at least one kill inside the apply, required), checks after every kill that `HEAD` and every changed path's file and index entry hold `H` or `U` (all `U` once `HEAD` is `U`), then resumes on a restarted host to `U` with a clean status and the deleted page in trash. A kill inside a host operation is finished by the restarted host first, so the resume may answer `pull-held` for a cycle or two (counted, never `diverged` or refused); a killed publication can leave its hidden temp file, which autocommit never stages (counted). Stated limits: D-20261028-GU (a pulled path edited back to exactly `H`'s bytes, then a crash before the ref moves, is re-applied; outside `gitSafety`); weak filesystems claim only `gitSafetyWeak` (R3); the barrier's folder syncs are the store witness without per-call containment, Windows flushes no folder (D-20261028-WDD); libgit2's internal multi-syscall windows (A-MUTATION-CONTAINMENT); no physical power-loss evidence. All eighteen listed tests passed the hosted runs L5 (Ubuntu 22.04, ext4 `/tmp` and runner temp), W5 (Windows Server 2025, NTFS) and M5 (macOS 26 arm64, APFS) at `9cd8ea830`; the twelve earlier ones also passed W4 and M4 at `335698374`, and the eleven before the status-scan fix W3 and M3 at `1d26d250f`. The kill test found one defect, fixed: the resume test committed a half-applied pull when its status scan failed (REG-SB1-GIT-RESUME-STATUS-ERROR, A-GIT-PULL follow-up; hosted run WX). A third finding, fixed: a failed read of one differing file in the same test committed too (REG-SB1-GIT-RESUME-HASH-ERROR). Promoted to `guarded+tested` for the recorded local ext4/NTFS/APFS scope on L5/W5/M5. Android/iOS: mobile-pending per SB2; Android shared storage refuses a case-only pulled rename (MediaProvider keeps the old spelling; a stated limit for beta.1, D-20261029-GS1; see A-CASE-DISTINCT); iOS compile only. Second finding, fixed ([A-GIT-PULL follow-up](#git-sync-sb1-gitsync-2026-10-29), D-20261029-GT, REG-SB1-GIT-TINE-OWN-TRASH): Tine committed its own trash copy, which stopped the next pull; `logseq/.tine-*` now stays out of git and out of the pull's comparison. | guarded+tested |

The unguarded items are tracked as follow-ups of the existing L9 audit work
(model-to-code link, milestone M2 onward):

F3 strengthens the test evidence around A-REFS-GIVEN and A-OPAQUE-TEXT:
`crates/tine-store/tests/conformance_f3.rs` compares generated real
Markdown/Org rename/merge bytes to an independent fixture renderer and live
indexes to a cold build. These remain assumptions of the formal host proof;
bounded semantic tests do not change their status. Generated retained commit
and restore conservation journeys live in `tests/transaction.rs`,
`tests/restore.rs` and `tests/i02_crash_gaps.rs` in the same crate. The latter's
OS aborts and the native adapter's pre-phase faults are separate from model
power cuts; the four evidence classes and their limits are listed in
[README.md](README.md#evidence-classes-and-semantic-conformance).

- A-ROOT-BOUND follow-up: Android FUSE stability and special desktop-volume acceptance; hosted local-volume runtime/cost evidence is recorded below.
- A-DRAFT-NAMESPACE follow-up: iOS app-update rebase device acceptance.
- A-ONE-HOST acceptance: D-20261027-OF warning, repair and handover retry; named hosted desktop account-lock, recovery and retirement tests passed. Mobile sandbox/exclusion acceptance remains pending. Failed arbitration stays an explicit claim limit.
- A-DIRSYNC follow-up: special desktop-volume and mobile identity/sync acceptance. Named hosted local-volume tests passed; Windows directory flush is not claimed. M3-1/M3-2 now have active passing regressions. M4 covers L9-13's existing bounded durable withdraw chain and primitive power outcomes; native power-loss refinement remains excluded.

## SB1 desktop acceptance (2026-10-28)

Martin decided the SB2 status clarification and DESK-SCOPE in D-20261028-DSC.
Desktop scope is local Windows NTFS, macOS APFS and
Linux ext4 volumes, plus only the witnessed Windows UNC behavior. Cloud
placeholders (OneDrive Files On-Demand and iCloud Drive) and other network/FUSE
mounts are **pending tester acceptance after beta.1 (DESK-SCOPE, decided D-20261028-DSC)**,
not accepted storage. Each affected Platforms cell makes that limit visible.

An executed `test ... ok` line counts within its actual assertion scope;
self-skipping native probes do not establish their skipped branch. Ignored
tests, absent cfg arms and zero-test filters do not count as executions. Duplicate executions are not extra scenario samples.

| Key | Receipt / tested commit | OS / runner | Storage actually identified in the log |
|---|---|---|---|
| L3 | [Linux run 40642454859](https://github.com/martinkoutecky/tine/actions/runs/40642454859), `dbdb8cb46277d4a761d7404a79980b5f885806a9`; saved `artifacts/ci/linux-40642454859.log` | Ubuntu 22.04.5 LTS; image `ubuntu-22.04` / `20261012.322.1` | Resolved app-data base, runner temp and temp base are ext4 on `/dev/nvme0n1p1` (lines 7982–7984). Three F-M3S-1 witnesses passed at runner temp (8080–8085) and resolved app data (8262–8267); four C6 witnesses passed at resolved app data (8172–8175). Cleanup is recorded for every F-M3S-1 sandbox. |
| L | The maintainer's local gate log, `4aa285dc66c4cfe34168a09fe9409c6f9fc6b38d` | Linux per manager; distribution/kernel/version not logged | Filesystem and backing-volume identity not logged. Named row tests passed as listed; all three F-M3S-1 witnesses were ignored. This cannot establish identified Linux local-volume acceptance. |
| W2 | [Windows run 40632086417](https://github.com/martinkoutecky/tine/actions/runs/40632086417), `8960039cc0544ab6adb59dbda23def3dfec8cfcc`; saved `artifacts/ci/windows-40632086417.log` | Windows Server 2025 Datacenter 10.0.26100; image `windows-2025-vs2026` / `20261021.278.1` | Tested-volume probes identify runner temp, `%TEMP%` and `%APPDATA%` as NTFS fixed drives (lines 4012–4014). Three F-M3S-1 witnesses passed at runner temp (4199–4204), resolved app-data base (4780–4785), and loopback UNC (4397–4402). Junction witnesses passed on local NTFS; they were not rerun under UNC. |
| WF | [Windows flush run 40624305046](https://github.com/martinkoutecky/tine/actions/runs/40624305046), `b31f594dd93f26ac139a8eb34905ebd49f78a75c`; saved `artifacts/ci/windows-40624305046.log` | Windows Server 2025, 10.0.26100; elevated hosted runner | No independent filesystem-type probe in this run. The three Windows-only flush tests executed/passed (lines 2332–2334): API success, open failures and injected post-open flush failure. Evidence is limited to API/error-policy behavior; no unprivileged or power-loss acceptance. The app-level namespace-sync test is absent and not credited. |
| M2 | [macOS run 40632088537](https://github.com/martinkoutecky/tine/actions/runs/40632088537), `8960039cc0544ab6adb59dbda23def3dfec8cfcc`; saved `artifacts/ci/macos-40632088537.log` | macOS 26.6.2 (25G83), arm64; image `macos-26-arm64` / `20261013.0424.1` | Resolved app-data base, runner temp and TMPDIR are on `/System/Volumes/Data`, APFS local/journaled (lines 1946–1948). Three F-M3S-1 witnesses passed at app data (2248–2253) and runner temp (2636–2641). Unicode/normalization and native self-alias witnesses passed on this APFS volume. |
| E | The maintainer's local gate log, earlier real-app run at `4b1e1bf6d14d6ef58e70ee1bffb82cbdc36c10ce` | Linux per manager; OS version and filesystem not logged | Owner warning/edit/repair/handover PASS 3/3 (lines 8242–8247); earlier evidence only, not a current-commit app run. |

L3 records Ubuntu version at lines 11–14 and checkout hash at 603/615.
The mount options include `nobarrier,data=writeback,journal_async_commit`;
these logical-fault and process-recovery tests supply **no physical durability
(power-loss) evidence**. No run claims native physical power-loss refinement.
The L3 job concluded failure because a frontend Vitest file hit the unrelated
`spawnSync rtk ENOENT` tooling leak. Rust core tests, tested-volume probes,
Linux volume acceptance and app-crate tests executed and passed; storage
conformance, the strace backup gate and later steps were skipped and are not
credited. A failed overall job is not recast as a green suite.

W2/M2 logs contain checkout hashes (W2 line 113, M2 line 109) and complete
cleanup. The earlier W/M runs at `4aa285dc6` remain historical evidence but
have no filesystem-type probes; W2/M2 supply the new NTFS/APFS evidence.

### What the UNC result establishes

W2 supplied `\\localhost\D$\a\_temp\unc-accept` (line 4222). The acceptance
harness canonicalizes the supplied root **before** creating the sandbox
(`storage_acceptance_tests.rs#on_volume`); all three PASS records retain
`\\?\UNC\localhost\D$\a\_temp\unc-accept` (4397, 4399, 4401). It did not
become a drive-letter path. `production_tests.rs#Fixture::in_empty_root`
joins graph/app/trash under that sandbox; `production.rs#ProductionIo::attach_bound`
retains those paths. Containment checks inspect resolutions but do not replace
I/O targets (`model/write_target.rs#checked_write_target`), and
`atomic_file.rs#native_path` preserves existing verbatim paths for native moves.

This is Tine publication, finite metadata interference with successful retry,
and physical draft recovery after cold reopen through the Windows loopback
administrative UNC share backed by local NTFS. It is bounded loopback SMB/UNC
I/O evidence, not acceptance of a remote server, disconnects/reconnects,
server durability, UNC Unicode/adoption/owner behavior, or F-DC-1 UNC
root/parent substitution refusal, custody and retry. The three ordinary-volume
witnesses do not exercise those UNC-specific faults.

Seven rows now meet the clarified desktop rule: A-PATH-IDENTITY,
A-ROOT-BOUND, A-DRAFT-NAMESPACE, A-ONE-HOST, A-DIRSYNC, A-DRAFT-DURABLE and
A-CASE-DISTINCT are
`guarded+tested` for the recorded local ext4/NTFS/APFS scope. L3 supplies the
previously missing Linux provenance and F-M3S-1 executions. Every applicable
pre-existing named Fault test passed L3/W2/M2; the three added Windows-only
flush tests passed WF. Platform-specific exclusions are explained
per row and are not credited as passes. The UNC fault gaps limit UNC scope to
the executed F-M3S-1 cases under DESK-SCOPE; they are not silently counted as
executed or imposed as extra local-volume tests. Android/iOS remain
mobile-pending under SB2. A-DRAFT-DURABLE is recorded under the decided Windows
limit D-20261028-WDD; Windows draft-folder creation/removal and fallback-rename
persistence remain unproven. A-MUTATION-CONTAINMENT stays `unguarded`.
Totals after SB1-GITSYNC (below) and A-GIT-PULL's promotion on L5/W5/M5:
fourteen `guarded+tested`, four `assumed-outside`, one `unguarded`.

## Git sync (SB1-GITSYNC, 2026-10-29)

Seen and git sync (ADR 0078 slices 2 and 3, ADR 0079) add these write paths.
Every one has a row in `crates/tine-store/tests/write_sites.toml`; i01 now scans
every production file that names `git2` for libgit2 mutators, which found the
two rows marked new (fail-before: i01 reported exactly those two owners).

| Path | Writes | Census rows | Ledger rows |
|---|---|---|---|
| Pull pages | Graph pages and their trash, as page-host operations | `git_pull.rs` `plan_pages`, `plan_rename` | A-GIT-PULL; through the host: A-ATOMIC-REPLACE, A-PATH-IDENTITY, A-ROOT-BOUND, A-CASE-DISTINCT, A-DELETE-ONE-MOVE, A-TRASH-COUPLED |
| Pull other files | Assets, `config.edn` and other non-page files, as store transactions | `git_pull.rs` `plan_other`, `apply_pull` (new) | as above |
| Pull barrier | fsync of changed paths and folders, no content | `git_pull.rs` `barrier` | A-GIT-PULL; folder sync: A-DIRSYNC witness |
| Index and ref | `.git/index` by lock and rename, branch compare-and-swap | `git_run.rs` `IndexLock::*`, `Repo::advance` | A-GIT-PULL, A-MUTATION-CONTAINMENT |
| Fetch | `.git/objects/pack` and the remote-tracking ref only | `git_sync.rs` `Operation::fetch_locked` (new) | A-MUTATION-CONTAINMENT |
| Autocommit (timed, pre-pull, quit, mobile background) | objects, index, branch; the `.gitignore` lines `.syncthing.*` and `logseq/.tine-*` | `git_autocommit.rs` rows, `git_run.rs` `Repo::tree_with_parts`, `commit_pair`, `run_end` | A-MUTATION-CONTAINMENT; deferral during a pull: A-GIT-PULL (P3) |
| CRLF setting | `core.autocrlf` in `.git/config` | `line_endings.rs` `disable` | A-MUTATION-CONTAINMENT |
| Push | nothing local (remote only) | none needed | — |
| Credentials, seen baseline | app data only (`git-credentials.json`, seen files) | `credentials.rs` `Tokens::*`, `seen.rs` `save`, `forget` (domain app) | outside the graph root; refusal rows in `docs/storage-contract.md` |
| Mobile lifecycle | nothing itself: page save, autocommit and `push_pending` above | none needed | as above |

A-GIT-PULL follow-up (found and fixed, D-20261029-GT, REG-SB1-GIT-TINE-OWN-TRASH):
autocommit staged Tine's own `logseq/.tine-trash/` copies. After any page
deletion (pulled or local) the trash copy was committed, so the next pull of a
new remote change was `diverged`; a trash copy another Tine device pushed made
this device's pull refuse (`pull-refused`, Tine's own folder). Now, like
`.syncthing.*` (D-20261026-GI): autocommit adds `logseq/.tine-*` to the graph
root `.gitignore` once per device (one write and one notice when both lines are
due), never stages those paths, and untracks ones already committed in a commit
of their own just before its next real change (the files stay on disk); a pull
leaves them out of `H` and `U` (`without_tine_own`), so it neither writes nor
refuses over them, and sets the index's own entries to `U`'s before the index
install (`own_entries_at`), so nothing is staged under the published `U`
(review round 1 R1: before, `git diff --cached` showed a synthetic deletion an
ordinary `git commit` would record; fail-before
`q_git_pull_skips_tine_own_paths_without_refusing`,
`q_git_own_entries_resume_across_index_and_ref`). Fail-before: both former probes
(`a_pulled_deletion_does_not_make_the_next_pull_diverge`: `diverged`;
`another_devices_trash_copy_does_not_stop_the_pull`: `pull-refused`) on
`e8942c758`. Stated limit: a device still running an older Tine keeps
committing its trash; the newer device pulls past it and untracks it with its
next change; the older Tine's own pull still refuses that untrack commit
(Tine's own folder) until it is updated. Autocommit is development-only, so
only development repositories can hold committed trash.

A-GIT-PULL follow-up (found and fixed, REG-SB1-GIT-RESUME-STATUS-ERROR): a
killed pull could resume to `diverged` ("this device had changes, now
committed"). The restarted host saves the killed operation's recovered pages;
when libgit2's status scan inside the resume test (`defers_autocommit`, P3)
read a file that save was replacing, the scan failed ("error reading file for
hashing") and the test answered "commit", so the pre-pull autocommit recorded
the half-applied tree on `H`. The local full suite reproduced it about once in
30 runs, with the scan error captured; hosted Windows run WX ended the same way
once in six runs (that run predates the diagnostic, so its cause is inferred
from the matching sequence). A failed scan now
defers the autocommit; `q_git_resume_test_that_cannot_scan_defers_the_autocommit`
fails before the fix (it commits) and passes after. Its sibling, a failed read
of one differing file in the same test, committed too; it now defers as well,
while a folder or link where a file was still counts as a local change
(REG-SB1-GIT-RESUME-HASH-ERROR, found by the beta.1 claim verifier;
`q_git_resume_test_that_cannot_hash_a_file_defers_the_autocommit` fails before
the fix). Still open: `pre_check` reports any read error as "a folder or link
is in the way" (a refusal, so nothing is written; only the message is wrong).

| Key | Run / commit | OS / runner | Storage identified in the log |
|---|---|---|---|
| W3 | [Windows 40818443748](https://github.com/martinkoutecky/tine/actions/runs/40818443748), `1d26d250f2f807bd7b554d63d37d81f7486cb7ca`; saved `artifacts/ci/windows-40818443748.log` | Windows Server 2025 Datacenter 10.0.26100; image `windows-2025-vs2026` / `20261021.278.1` | runner temp, `%TEMP%` and `%APPDATA%` NTFS fixed drives (lines 4068–4070). The eleven A-GIT-PULL tests passed (5708–6008); the kill test's graph is under `%TEMP%`. The overall job failed on an unrelated store test (`l9_all_transaction_entry_paths_refuse_a_detached_graph`, Access is denied in its setup; it passed the other Windows runs). |
| M3 | [macOS 40824254939](https://github.com/martinkoutecky/tine/actions/runs/40824254939), `1d26d250f2f807bd7b554d63d37d81f7486cb7ca`; saved `artifacts/ci/macos-40824254939.log` | macOS 26.6.2 (25G83), arm64; image `macos-26-arm64` / `20261013.0424.1` | app data, runner temp and TMPDIR on `/System/Volumes/Data`, APFS local/journaled (lines 2070–2072). All steps green; the kill test passed with 13 kills inside the apply (line 1788). |
| W4 | [Windows 40843055503](https://github.com/martinkoutecky/tine/actions/runs/40843055503), `335698374f8c36c557961a90bd570fffe0d9ef81`; saved `artifacts/ci/windows-40843055503.log` | Windows Server 2025 Datacenter 10.0.26100; image `windows-2025-vs2026` / `20261021.278.1` | runner temp `D:\a\_temp`, `%TEMP%` and `%APPDATA%` NTFS fixed drives (lines 4040–4042). All twelve A-GIT-PULL tests passed (5758–6056); every job step green. |
| M4 | [macOS 40843058620](https://github.com/martinkoutecky/tine/actions/runs/40843058620), `335698374f8c36c557961a90bd570fffe0d9ef81`; saved `artifacts/ci/macos-40843058620.log` | macOS 26.6.2, arm64; image `macos-26-arm64` / `20261013.0424.1` | app data, runner temp and TMPDIR on `/System/Volumes/Data`, APFS local/journaled (lines 1345–1347). All twelve tests passed; the kill test had 12 kills inside the apply and 7 held resumes (line 1058); every step green. |
| L5 | [Linux 40889341558](https://github.com/martinkoutecky/tine/actions/runs/40889341558), `9cd8ea830fedfd927f2a593b4e42912d9072e1c6` | Ubuntu 22.04; image `ubuntu-22.04` / `20261025.340.1` | `/tmp`, runner temp and app data on ext4 `/dev/nvme0n1p1` ("Linux tested volumes" step). All eighteen A-GIT-PULL tests passed in "App crate tests". The job's "Performance baseline policy" step failed (the policy still named v0.8.1 after 0.8.2 shipped; fixed in the release prep), outside the storage steps. |
| W5 | [Windows 40889346114](https://github.com/martinkoutecky/tine/actions/runs/40889346114), `9cd8ea830fedfd927f2a593b4e42912d9072e1c6` | Windows Server 2025; image `windows-2025-vs2026` / `20261021.278.1` | runner temp `D:\a\_temp`, `%TEMP%` and `%APPDATA%` NTFS fixed drives. All eighteen A-GIT-PULL tests passed; every job step green. |
| M5 | [macOS 40889351215](https://github.com/martinkoutecky/tine/actions/runs/40889351215), `9cd8ea830fedfd927f2a593b4e42912d9072e1c6` | macOS 26, arm64; image `macos-26-arm64` / `20261021.0445.8` | app data, runner temp and `$TMPDIR` on the APFS data volume (local, journaled). All eighteen A-GIT-PULL tests passed; every job step green. |
| WX | [Windows 40811643673](https://github.com/martinkoutecky/tine/actions/runs/40811643673), `291a9e68bf53496058d5d1c2f89b68b07846ce48` | as W3 | NTFS; the kill test failed with the defect described above (line 6337), before the fix. |

Further runs of the same test: Windows [40815985981](https://github.com/martinkoutecky/tine/actions/runs/40815985981)
(`5e2ca45a9`, all green), [40824214900](https://github.com/martinkoutecky/tine/actions/runs/40824214900) and
[40824225687](https://github.com/martinkoutecky/tine/actions/runs/40824225687) (`1d26d250f`; kill test passed, other
pre-existing Windows tests failed once each: a push reported `network`, an HTTPS cancel server saw no POST, a store
binding timing test); macOS [40815988624](https://github.com/martinkoutecky/tine/actions/runs/40815988624) (`5e2ca45a9`,
kill test passed; a store binding timing test failed once). The first macOS run, 40811645268, failed on a bisection
overflow in the test itself, fixed in `5e2ca45a9`.

## Mutation ledger

The A-MUTATION-CONTAINMENT follow-up records the M1 census findings; it is
part of the same follow-up work, not a production fix in M0/M1:

- `crates/tine-store/src/bin/export-block-raws.rs#main` overwrites
  `<supplied graph>/block-raws.json` directly. It has no containment or atomic
  save guard. This is a developer export tool, not the application save path.
- `transaction.rs#PagePlan::fault_collision` and `PagePlan::fault_twin`
  have no local containment guard. They simulate external writes, gated by
  `fault()` (false without test-faults); they remain syntactically present in
  production, so they are recorded conservatively rather than skipped.
- Application Git operations now retain graph/worktree witnesses and witnesses for Git's
  selected gitdir/commondir. The worktree's `.git` marker cannot be a directory
  alias; ordinary linked-worktree/submodule pointer files retain their selected
  external metadata scope. `GitMutationGuard` reuses Store's target-containment
  and platform root-identity policy. Runs retain the original Store; autocommit
  workers retain it through their final unbind commit. Index acquisition,
  installation, stale-copy cleanup, Drop cleanup, pack publication, loose-object
  deletion and directory sync check their actual targets. Libgit2 object,
  tree, commit, ref and index writes recheck metadata scope before invocation;
  object fanout checks inspect existing two-hex directories, never history or
  payloads. Changed bindings refuse with GraphRootMoved; unsafe cleanup leaves
  locks/copies for inspection rather than deleting through the replacement.
  Sequential check-to-call races, libgit2's internal multi-syscall windows,
  Android's vanish-only fallback and weak/reused identities remain. Tests cover
  root and metadata-root replacement and redirected index/pack/object paths on
  Linux, and passed hosted Windows (junction fixtures) and macOS runs W3/M3
  (A-MUTATION-CONTAINMENT row; the index-file symlink witness is Unix-only).
  These operations write no page data and remain outside the model oracle.
- Fetch (`git_sync.rs#Operation::fetch_locked`, SB1-GITSYNC) holds
  `index.lock`, downloads exactly the tracked branch through an anonymous
  remote into `.git/objects/pack` and moves only the remote-tracking ref by
  compare-and-swap; `check_git` runs before the download and again after the
  network I/O. It writes no `FETCH_HEAD`, tag, `HEAD`, index or work-tree file.
  libgit2's pack write itself is not containment-checked per syscall.

### Windows draft durability limit (SB1 / G5; D-20261028-WDD)

Microsoft's [MoveFileExW documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
documents a write-through move. Tine already attempts that flag for draft
replacement, but falls back to ordinary rename on failure.
[FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
requires write access and documents volume-wide flushing with administrative
privilege; [Directory Handles](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)
documents backup-semantics handles without promising directory-entry persistence.
Those Win32 sources do not establish an unprivileged general directory flush for
the existing create/rename/unlink draft protocol. The driver-only
[NtFlushBuffersFileEx documentation](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex)
is not a supported user-mode implementation recipe here.

Tine now also attempts a best-effort directory flush at the private (app-data)
directory-sync boundary, including draft mkdir parents and unlink parents (graph
folders are unchanged: they may be network or cloud folders, where a flush per
save would add a server round trip):
`CreateFileW(GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS)`, then `FlushFileBuffers` and close.
All open/flush errors retain the previous success result. The Windows storage report's `windowsDirectoryFlush`
counts API successes/failures across the process since launch, without paths;
success is not evidence of a flush on every filesystem.
This best-effort app-data hardening accepts a latency risk: the synchronous
`CreateFileW(GENERIC_WRITE)` can wait for a conflicting oplock break acknowledgment
even with share-all. Microsoft's [oplock create rules](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/irp-mj-create2)
describe the conflicts; [Breaking Oplocks](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/breaking-oplocks)
documents no timeout for the acknowledgment wait, so there is no finite bound.
Tine adds no timeout/cancellation, and swallowing errors preserves results only
after I/O completes. App data may itself be redirected or cloud-backed; provider
interaction and flush latency remain unverified. This risk is accepted for this
app-data attempt, with graph folders unchanged.
[MS-FSA §2.1.5.7](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/0de7dc40-9627-437e-a4df-c4696cdc3d02)
specifies directory-structure persistence, but
[product behavior note <80>](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4e3695bd-7574-4f24-a223-b4679c065b63)
limits that implementation to NTFS; other filesystems may return success while
doing nothing. This does not establish a supported Win32 directory durability
contract or prove unprivileged operation or power-loss persistence. Hosted
Windows runners run elevated; a passing fresh-directory flush test is only API
acceptance there, not evidence for unprivileged users.

Martin decided in D-20261028-WDD that A-DRAFT-DURABLE may be recorded with this
stated Windows limit. Windows draft-folder creation/removal and fallback-rename
persistence remain unproven: the best-effort attempt supplies no unconditional
crash guarantee. The passing WF flush tests establish API/error-policy behavior
only; W2 cold recovery establishes process-recovery behavior, not namespace
persistence after power loss. Under SB2/DESK-SCOPE, A-DRAFT-DURABLE is
`guarded+tested` for the named desktop obligations within this limit, with
Android/iOS mobile-pending. Volatile-store detection is advisory on all five
targets, never a graph-open refusal or proof that every unflagged store is durable.

The shared `checked_write_target` now checks a process-local root witness;
graph mkdir callers pass a bound and create descendants one level at a time.
`Graph::withdraw_file_to_conflict_if` uses that bounded durable creation too.
Restore now checks the same binding at entry/reservation and each file move or
publication. ModelFs adds Bound/Gone/Replaced epochs and root faults between
modeled syscalls; native conformance replays the graph phase-boundary schedules.
The interior native directory-creation race is covered separately. The model
extension is executable adapter evidence, not a change to the formal s3.5 model,
its hash or proof claim. M4's withdraw evidence remains a primitive/API witness,
not full transaction refinement or native physical power-loss evidence.

`crates/tine-store/tests/write_sites.toml` classifies every syntactically found
production mutation owner in tine-store, tine-graph-features, src-tauri Rust,
and the Swift mutation fallback. i01 walks source trees rather than a fixed
file list, rejects missing/stale rows and changed primitive counts, resolves
guard/test names, pins create-root and container exceptions, and checks oracle
rows against ProductionIo's HostIo methods in `io.rs`. Own primitives are
counted along with raw writes, namespace mutations and syncs. A primitive's
body has its own row; its caller's domain supplies the target policy.

Rows marked `unguarded:A-ID` are findings, not claims of containment. Each
must point to an unguarded assumption here. Classification and syntactic guard
presence remain reviewed evidence: the census provides no taint analysis, call
graph or ordering proof. S4's separate cfg(test) RG7 hook checks a matching
lexical scope at mutation primitives, not root/parent freshness. HostIo helpers have `oracle=false`;
only actual mutating seam methods have `oracle=true`. The oracle still does
not cover transactions, restore, publish, purge or git (environment writes).
M2 adds test-only directory epochs/kinds and graph-key parents beside ModelFs's
byte maps. The driver observes disk/stable through executable α and rejects a
nonempty escape history at every comparison and step entry. C1/C2 schedules
cover the modeled syscall slots, nested-page mkdirs, and app-data independence;
full save projections compare environment deletions and host failures against
the unchanged s3.5 oracle. No directory behavior is formally proven.

A-PATH-IDENTITY and A-ROOT-BOUND are `guarded+tested` for the desktop scope
recorded above; special-volume and mobile acceptance remain pending. M2 adds
bounded fault evidence, not a general native no-escape proof. M2-1 (root replacement,
on identity-checking targets) and M2-2 (outside parent symlink) between planning
and finish are active passing regressions, with unchanged outcome assertions.
All platforms recheck the original graph binding and actual parent containment
before every path-based mkdir and parent sync. A swap/move after the final
check can redirect an empty mkdir outside. Retained sync debt checks the created
entry and parent identities before and after sync; detected change reports the
old creation non-durable, then retry re-plans and syncs the entire current chain.
Sequential probes can miss a swap and restoration; Android remains vanish-only
and relaxed SMB/FUSE/reused identity limits remain. Original binding capture and ordinary
file-publication races remain. Android's existing vanish-only policy is preserved;
M2-1 replacement detection is not claimed there. No plan holds directory handles.
The graph adapter retains failed plans with parent debt or missing components
until successful completion or adapter close. Re-planning replaces stale
witnesses with current-chain obligations, including parent syncs owed once
missing components are created or encountered as AlreadyExists after restoration.
This custody is process-local to the retained host plan, not a persistent debt
journal or retention by one-shot helpers.
Native directories are tested against the model, never claimed proven; ModelFs's
ideal per-syscall guard does not establish those native residuals are absent.
The existing RG7 test-only checked-target witness is reused, not replaced.
`crates/tine-store/src/page_host/production_guard_tests.rs#dircreate_swap_during_save_keeps_input_in_durable_device_draft`
injects root/parent changes just before the fresh creation recheck, then recovers
the refused input from the ordinary durable device draft.
M3 adds bounded NativeFs C1/C2 replay through the real ProductionIo adapter,
with independently resolved native α, empty outside sentinels and preserved
drafts. C3/C4 before the base guard are tested against the model in all four
profiles and sampled on native; C6 error classes test failed saves and draft
admission. The transaction primitive probes check only escapes (`oracle=false`).
The committed native corpus still asserts exactly 158 scenarios / 618 profile
outcomes and keeps its original exclusion/refinement ledgers. A separate M3
ledger names Windows C2, Android same-path root replacement, interior native
syscalls and native power-loss exclusions. The symlink harness executes on
Linux/macOS/iOS/Android; hosted/device acceptance is separate.

The test-only replay driver maps a scripted successful flush to the existing
`dirSync(false)` successor when the independently recorded external epoch or
physical bytes show that the published file changed. It does not inject that
failure: the real adapter must reject the stale publication. The negative
control `conformance_tests.rs#m3_identity_refinement_requires_the_adapter_to_refuse`
rejects an adapter that accepts the changed file. The s3.5 model, hash, guards,
full observed-state/guarantee comparisons and corpus counts are unchanged.

Within that qualified desktop evidence, M3-1 checks the publication handle's
stamp against one post-sync file metadata probe. Missing/changed/uncheckable
publication identity takes Uncertain and preserves input risk, including after
Unsupported sync and Windows's no-op sync. NotFound is no longer Unsupported.
M3-2 binds retained debt to the entry and its parent using the existing directory
identity policy; retry after detected replacement syncs the current chain rather
than acknowledging the detached epoch. Missing ancestors at the pre-mkdir check
or mkdir request the same re-plan, including partial creation and an interrupted
re-plan. Refused attempts retain the current parent and missing-component queues;
replacement queues are constructed before installation. Even a replacement with
only missing components is retained before its first mkdir records parent debt.
The native
`directory_durability.rs#dircreate_partial_missing_ancestor_recovers` and
`directory_durability.rs#dircreate_interrupted_replan_recovers` regressions (and
their `_at_mkdir` counterparts) require one stable retry without external repair
or graph reopen. The ModelFs twin
`page_host/native_environment_tests.rs#m3_model_partial_directory_creation_recovers`
covers its combined parent-check/mkdir step; it rescans missing components on
each attempt rather than retaining a native missing queue. The separate native
host regression
`page_host/production_tests.rs#m3_partial_replan_refusal_retains_root_sync_until_restoration`
moves an ancestor, refuses an outside substitution during a zero-parent re-plan,
then restores the ancestor. Both refused saves retain input risk and perform no
sync; the next save through the same host must sync the original graph root
before it reports Published and clears risk. This is Linux runtime evidence of
sync custody, not a native power-cut test. The original binding,
containment and input-custody refusals remain. Both findings are active passing
regressions with unchanged outcome assertions on identity-checking test storage;
the native M3-2 replacement case excludes Android's approved vanish-only policy.
Windows publication stamps use creation time, while directory witnesses use full
FILE_ID_INFO. Weak/reused identities and swaps restored between sequential
probes remain; no exact original-parent flush is claimed for those windows.
Directory behavior is tested against the model, never proven by it.
C5/C7/C8 and the withdraw C6 power variant remain M4 work. The s3.5 external
write guard also cannot project a stable-only disappearance when disk is
already ABSENT; trash-sync environment schedules currently check physical α
and escapes, not full oracle refinement of that case.

L9 per edit adds zero bytes/files/transport. Root checks add metadata calls at
graph I/O boundaries (six root checks plus one post-sync file metadata probe for
both 1- and 60-block measured native saves, formerly six plus zero), and
open-time namespace discovery is bounded by the device graph list. See receipt.
Neither the root binding nor directory-creation plans hold directory fds;
SMB/CIFS
(noserverino) and FUSE filesystems reissue inode numbers after eviction, and
there only device and birth time are compared. Linux statx btime (compared when
both sides have it) reduces the inode-reuse window; Apple compares device/inode
only, since utimes can move st_birthtime back. Delete/recreate with reused inode
and indistinguishable birth time can still evade identity comparison. Android is deliberately
vanish-only pending shared-storage FUSE device verification.

Real-window conformance coverage (F1/F6):
`src/document/host/nativeConformance.test.ts` drives the actual TS wiring/client
against `binding_window_tests.rs#native_window_peer`, a test-only native
Store/PageHost transport. A bounded open/edit/send/receive/publication prefix
uses the existing Quint-derived oracle. Further seeded lifecycle and publication
journeys check custody, serialization, newer input, session isolation and
live frontend mirrors against independent cold native answers. See README's
trust-gap scope for the exact boundary: no literal Tauri IPC or full random
Quint/application lockstep is claimed. A-OPAQUE-TEXT remains assumed outside
the proof; this adds bounded serialization evidence without changing any
assumption status, production write site or per-edit cost.

Physical conformance coverage (F3a/F7):
`crates/tine-store/src/page_host/scheduler_retirement_tests.rs#whole_vehicle_retirement_every_physical_fault_and_recovery_cut`
checks production grouped retirement against individual model removals at
physical barriers and crash/power recovery cuts.
`crates/tine-store/src/page_host/scheduler_random_tests.rs#scheduler_random_walks_with_faults_and_disabled_actions`
counts grouped retirement, current/stale-plan apply with absent/present effects,
claimant arrival/removal, switch completion/refusal, external deletion and all
three-page power survivor subsets (including actual keep/revert differences).
These tests strengthen the existing ModelFs link; they do not change any
assumption's status, add native power-cut evidence, or establish A-PROGRESS.
