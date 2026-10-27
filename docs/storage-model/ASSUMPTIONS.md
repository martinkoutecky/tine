# Storage model assumptions and mutation scope

This is the M0–M4 ledger for storage s3.5. It records the boundary of the
existing evidence; it adds no formal claim. Model anchors are literal substrings
of `quint/storage-s35.qnt`. Code references use repository-relative `file#owner`;
test references name actual `#[test]` functions. The meta-test connects the
model bytes to the existing transcription/replay hash (G8) and checks all named
anchors, guards and tests. An existing test listed on an incomplete row covers
only the scope described below, not its pending environment fault.

Statuses: `guarded+tested` means the stated scope has a guard and existing fault
evidence; `guarded-untested` means a guard exists but the full assumption lacks
the required fault evidence; `unguarded` means at least the named scope lacks a
guard; `assumed-outside` means excluded from this storage proof. Guard existence
does not establish check ordering or root identity.

| ID | Assumption | Model anchor | Guard (code) | Fault test | Platforms | Status |
|---|---|---|---|---|---|---|
| A-PATH-IDENTITY | Containment and original root binding are rechecked; publication metadata is checked after sync and retained debt checks entry/parent identity; sequential native races remain | `disk: int -> int` | `crates/tine-store/src/page_host/production_guard.rs#ProductionIo::check_path`<br>`crates/tine-store/src/page_host/production.rs#ProductionIo::page_sync`<br>`crates/tine-store/src/directory_durability.rs#DirectoryCreation::finish` | `crates/tine-store/src/page_host/production_guard_tests.rs#l8_01_actual_io_rechecks_after_admission_and_after_temp`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_save_schedules_refine_alpha_and_keep_durable_input`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_syscall_cross_product_including_trash_and_app_data`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_finding_c2_symlink_between_directory_plan_and_finish`<br>`crates/tine-store/src/directory_durability.rs#dircreate_rechecks_each_actual_parent_before_mkdir`<br>`crates/tine-store/src/directory_durability.rs#dircreate_sync_refusal_retains_unpaid_debt_until_safe_retry`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c1_c2_native_replay_matches_model_alpha_and_keeps_input`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c3_c4_native_and_model_do_not_publish_a_stale_recreation`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_parent_loss_after_publish_must_not_report_current_durable`<br>`crates/tine-store/src/transaction_environment_tests.rs#m3_transaction_apply_and_apply_move_c2_c3_escape_only` | M2-2 and M3-1/M3-2 are active passing bounded regressions on identity-checking test storage. All five targets check binding/containment before mkdir and parent sync; retained debt checks entry and parent identities before/after sync. A detected replacement reports non-durable creation and retry re-plans the current chain. The retained host plan includes missing-component obligations even before any replacement mkdir records parent debt. Post-sync file metadata checks the publication stamp without payload I/O. Android directory identity stays vanish-only; weak/reused identities and swaps restored between sequential probes remain. An ancestor swapped/moved after the last relevant check can still redirect an empty mkdir outside; ordinary file publication retains its check/OS-call race. Linux executed; Windows C2 explicitly ModelFs-only and platform runtime remains pending. Directories are tested against the model, never proven. | guarded-untested |
| A-ROOT-BOUND | Application graph writes check the directory bound at open; Tine never creates the graph root or an ancestor | `disk: int -> int` | `crates/tine-store/src/root_binding.rs#RootBinding::check`<br>`crates/tine-store/src/directory_durability.rs#DirectoryCreation::new`<br>`crates/tine-store/src/directory_durability.rs#DirectoryCreation::finish` | `crates/tine-store/src/page_host/production_guard_tests.rs#l9_replaced_root_refuses_creating_save_and_keeps_input`<br>`crates/tine-store/src/directory_durability.rs#l9_directory_creation_never_recreates_a_vanished_bound`<br>`crates/tine-store/src/restore.rs#l9_restore_refuses_a_replaced_root_before_reserving_recovery`<br>`crates/tine-store/src/page_host/native_conformance_tests.rs#l9_native_and_model_refuse_the_same_detached_root_phases`<br>`crates/tine-store/src/page_host/binding_root_tests.rs#l9_input_typed_after_root_moves_reaches_durable_custody`<br>`crates/tine-store/src/root_binding.rs#l9_root_binding_holds_no_directory_descriptor`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_save_schedules_refine_alpha_and_keep_durable_input`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_c1_c2_syscall_cross_product_including_trash_and_app_data`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m2_finding_c1_replaced_root_between_directory_plan_and_finish`<br>`crates/tine-store/src/directory_durability.rs#dircreate_existing_directory_adds_no_binding_or_sync_work`<br>`crates/tine-store/src/page_host/binding_root_tests.rs#dircreate_failed_mkdir_discard_close_releases_graph_handles`<br>`crates/tine-store/src/page_host/binding_root_tests.rs#dircreate_new_folder_save_needs_no_root_read_permission`<br>`crates/tine-store/src/directory_durability.rs#dircreate_checks_original_open_binding_even_if_replaced_before_planning`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c1_c2_native_replay_matches_model_alpha_and_keeps_input`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_native_m2_1_replaced_root_between_plan_and_finish` | M2-1 is active on identity-checking targets with unchanged outcome assertions. Every mkdir reuses the original open-time RootBinding; Linux/macOS/iOS keep existing stat/statfs policy, Windows FILE_ID_INFO, Android vanish-only (same-path replacement remains undetected). SMB/FUSE relaxed identities and inode reuse/unavailable birth time retain their existing limits. No directory handles or new read permission requirement. The host retains failed plans with recorded parent debt or missing components across Discard/page Close and retries until successful completion or adapter close. Re-planning transfers custody to the current chain's parent debt and missing-component obligations. Honest contained-parent replacement can retry after re-planning. Initial open-time capture, final check-to-create, parent rename/check-to-sync, and ordinary file-publication windows remain; no native no-escape proof. ModelFs tests alpha/escapes/draft custody; M3 bounded native phase samples (Linux executed) check α, escapes and durable input retention, with Android same-path replacement excluded from native replay; hosted platform acceptance remains pending. Developer export and git remain outside. | guarded-untested |
| A-DRAFT-NAMESPACE | After a root path change, pre-crash drafts are restored or surfaced for explicit adoption | `action launch` | `src-tauri/src/root_identity.rs#find_orphans`<br>`src-tauri/src/root_identity.rs#adopt_namespace` | `src-tauri/src/root_identity.rs#relocation_is_idempotent_and_preserves_sealed_bytes`<br>`src-tauri/src/root_identity_m4_environment_tests.rs#m4_c5_native_moved_graph_surfaces_every_old_namespace_vehicle`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c5_model_relaunch_at_new_path_surfaces_then_recovers_every_draft`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c5_native_moved_root_keeps_physical_orphans_and_recovers_after_adoption` | C5 composes app census/adoption with cold adapter recovery of existing, absent and changed-base records. Desktop relocation surfaces an orphan before explicit adoption; it never merges automatically. Known/unknown old roots and occupied targets are tested. Existing formats and validated iOS relocation remain; iOS device rebase and other hosted runtime acceptance remain unverified. | guarded-untested |
| A-ONE-HOST | One host owns a graph and its drafts | `alive: bool` | `src-tauri/src/owner/lease.rs#claim`<br>`src-tauri/src/state.rs#GraphRegistry::draft_namespace_held` | `src-tauri/src/root_identity.rs#old_graph_id_uses_the_same_owner_lock_as_the_host`<br>`src-tauri/src/host_retirement.rs#l9_retiring_source_refuses_draft_recovery`<br>`src-tauri/src/host_retirement.rs#l9_recovery_and_relocation_exclude_both_bound_and_retiring_namespaces` | Linux/macOS/Windows account locks; iOS/Android per-app sandbox cross-process exclusion. On all five targets registry/retirement entries exclude both draft namespaces, except the saved-stop recovery target. Lock errors retain diagnosed fail-open; recovery refuses without the current desktop lease. Windows runtime remains unverified. | guarded-untested |
| A-DIRSYNC | Directory sync settles names under the platform identity policy; publication and retained-debt checks reject detected parent loss/replacement | `action dirSync` | `crates/tine-store/src/directory_durability.rs#DirectoryCreation::finish`<br>`crates/tine-store/src/page_host/production.rs#ProductionIo::page_sync` | `crates/tine-store/tests/directory_durability_guard.rs#directory_sync_has_one_owner_and_no_discarded_result`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_parent_loss_after_publish_must_not_report_current_durable`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_c4_retained_directory_sync_must_not_acknowledge_a_new_inode`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_finding_c4_model_retained_directory_sync_must_not_acknowledge_a_new_inode`<br>`crates/tine-store/src/page_host/conformance_environment_tests.rs#m3_c6_classified_errors_refine_save_fail_and_failed_dirsync`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c6_native_withdraw_witnesses_durable_chain_and_same_volume_move` | M3-1/M3-2 active and passing on Linux; tested against the model. NotFound fails sync; changed publication stays Uncertain. Stale directory debt reports non-durable creation, then retry syncs the current chain. Missing components retain host custody through a refusal before the first replacement mkdir; restoring an earlier ancestor still requires its parent sync. Android directory checks stay vanish-only (native M3-2 replacement excluded); Windows performs identity checks without directory flush. Weak/reused identity, sequential substitution/restoration and hosted/device runtime remain open. M4 verifies withdraw's existing bounded durable chain on fresh/existing conflict trash and ModelFs power outcomes under the same-volume single-move recipe; no native power-loss claim. | guarded-untested |
| A-ATOMIC-READ | Reads return a whole version; arbitrary torn reads remain outside the model | `disk: int -> int` | `crates/tine-store/src/page_host/save.rs#Host::advance_save_locked` | `crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c7_model_partial_external_reads_conflict_without_publishing`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c7_native_in_place_writer_during_read_conflicts_and_keeps_input` | C7 tests empty/interior/almost-complete prefixes at Open and save Check; a barrier-controlled native thread truncates/writes after reader open/stat. Samples conflict, retain input durably and never publish. No atomic-read guard, arbitrary overlapping writers or final-check/rename race guarantee is inferred. Native Linux executed; other hosted targets pending. | assumed-outside |
| A-DRAFT-DURABLE | App-data drafts are durable on a strong backing store | `drafts: int -> Draft` | `crates/tine-store/src/page_host/production.rs#ProductionIo::draft_sync` | `crates/tine-store/src/page_host/native_conformance_tests.rs#native_committed_scenarios_in_four_profiles`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c6_errno_classes_keep_sticky_failure_and_admitted_input`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c6_draft_failed_matches_i8_host_apply_row`<br>`crates/tine-store/src/page_host/native_environment_tests.rs#m3_c6_errno_notice_stays_sticky_until_publication_on_both_backends` | C6 StorageFull/EIO-class Other/PermissionDenied/ReadOnlyFilesystem: admitted text survives failed saves in drafts, reattach and retry; draft failures refuse a new operation whole (I-8 host apply row); the actual error notice persists through drafting/idle reconciliation until publication. Linux executed; other platform runtime acceptance remains separate. Windows private directory sync and tmpfs classification L9-15 remain gaps. | guarded-untested |
| A-CASE-DISTINCT | Distinct page keys name distinct files, subject to physical spelling aliases | `pure val PAGES` | `crates/tine-store/src/no_replace.rs#move_file_noreplace`<br>`crates/tine-store/src/transaction/preflight.rs#PagePlan::page_create_checks` | `crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c8_model_case_alias_is_seen_by_admission_and_never_overwritten`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c8_native_case_insensitive_volume_admission_refuses_collision` | C8 ModelFs Case profile covers ASCII aliases, registered spellings, collision refusal and unchanged incumbent bytes. Native test probes the actual volume; case-sensitive volumes explicitly skip as NATIVE_MODELFS_ONLY. Windows/default macOS case-insensitive runtime acceptance and general Unicode alias coverage remain pending. | guarded-untested |
| A-ATOMIC-REPLACE | Publication is atomic old-or-new under the platform recipe | `action rename` | `crates/tine-store/src/atomic_file.rs#rename_replace` | `crates/tine-store/tests/internal_crash_transactions.rs#transaction_kill_reopen_preserves_each_old_or_new_file`<br>`crates/tine-store/tests/i16_platform_arms.rs#host_publication_witnesses_cover_exactly_the_five_shipped_targets` | All five recipes pinned; weak filesystems retain weaker durability | guarded+tested |
| A-DELETE-ONE-MOVE | Delete moves once into same-volume trash without replacing a target | `action rename` | `crates/tine-store/src/no_replace.rs#move_file_noreplace` | `crates/tine-store/src/page_host/native_conformance_tests.rs#native_committed_scenarios_in_four_profiles` | All five; unsupported rename-noreplace fallback has accepted creator race; errno-class test pending | guarded+tested |
| A-TRASH-COUPLED | A durable page's bytes survive deletion or withdrawal in its page or trash payload | `trashStable: int -> Set[int]` | `crates/tine-store/src/page_host/production.rs#ProductionIo::trash_sync`<br>`crates/tine-store/src/model.rs#Graph::withdraw_file_to_conflict_if` | `crates/tine-store/src/page_host/conformance_tests.rs#all_s3_scenarios_in_four_profiles`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c6_withdraw_model_power_cuts_keep_page_or_conflict_payload`<br>`crates/tine-store/src/page_host/m4_environment_tests.rs#m4_c6_native_withdraw_witnesses_durable_chain_and_same_volume_move` | Existing page-host oracle power cuts plus M4's six withdrawal primitive power outcomes: durable chain and same-volume atomic move retain old source or conflict payload (surplus copy allowed). Native verifies chain sync requests and exact displaced bytes, not physical power loss. Full transaction/withdraw refinement, weak filesystems and external unflushed payload data remain outside this added scope. | guarded+tested |
| A-NAME-SLOT | Creation checks another claimant of the same logical name | `val claimedCreates` | `crates/tine-store/src/page_host/production_guard.rs#ProductionIo::creating_owner` | `crates/tine-store/src/page_host/production_guard_tests.rs#l8_02_raw_creation_rules_survive_failure_and_new_input_revokes_them` | All five; plain creating-save model admission JA5/s3.6 pending | guarded-untested |
| A-EXT-WHOLE | Environment writes replace whole files | `action extWriteD` | — | — | All five; in-place writes excluded, see A-ATOMIC-READ | assumed-outside |
| A-PROCESS-CUT | Process/window may die between syscalls | `action crash` | `crates/tine-store/src/atomic_file.rs#PreparedWrite::with_hooks` | `crates/tine-store/tests/internal_crash_transactions.rs#transaction_kill_reopen_preserves_each_old_or_new_file` | Page-host phase cuts and selected subprocess journeys; not native power-loss refinement (G7) | guarded+tested |
| A-REFS-GIVEN | Referrer discovery is supplied correctly | `action opRename` | — | — | All five; index/referrer correctness outside storage proof | assumed-outside |
| A-OPAQUE-TEXT | Texts abstract bytes as small integers | `Texts are small integers` | — | — | All five; serialization/round-trip obligations SPEC-s2 §4.1–4.6 outside model | assumed-outside |
| A-PROGRESS | Scheduling eventually runs the host | `action draftSync` | `crates/tine-store/src/page_host/driver.rs#Shared::submit` | `crates/tine-store/src/page_host/driver_tests.rs#every_admission_wakes_a_sleeping_driver` | All five; default Progress replay (G3) pending | guarded-untested |
| A-MUTATION-CONTAINMENT | Every graph-derived mutation has a containment guard; M1 findings below are exceptions | `disk: int -> int` | — | — | Developer export CLI, test-injection helpers, and .git path operations | unguarded |

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

- A-ROOT-BOUND follow-up: Android FUSE stability and Windows runtime/cost acceptance.
- A-DRAFT-NAMESPACE follow-up: iOS app-update rebase device acceptance.
- A-ONE-HOST follow-up: visible warning on diagnosed fail-open (L9-12).
- A-DIRSYNC follow-up: hosted/mobile identity and sync acceptance. M3-1/M3-2 now have active passing regressions. M4 covers L9-13's existing bounded durable withdraw chain and primitive power outcomes; native power-loss refinement remains excluded.

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
- `src-tauri/src/owner/git_run.rs#IndexLock::acquire`, `IndexLock::install`,
  `IndexLock::drop`, `remove_stale_copies`, and
  `src-tauri/src/owner/git_autocommit.rs#repack`, `remove_object`, `sync_dir`
  derive paths from the repository's `.git` location or their caller. Their
  git locks/publication rules do not supply the ledger's qualifying graph
  containment/root binding guard. They do not write page data.

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

A-PATH-IDENTITY and A-ROOT-BOUND remain `guarded-untested`: M2 adds bounded
fault evidence but does not complete their stated scope. M2-1 (root replacement,
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

These rows retain incomplete statuses: M3-1 now checks the publication handle's
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
