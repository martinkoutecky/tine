# Storage model assumptions and mutation scope

This is the M0/M1 ledger for storage s3.5. It records the boundary of the
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
| A-PATH-IDENTITY | A page path keeps its file identity; containment is rechecked, root replacement and check-then-act remain gaps | `disk: int -> int` | `crates/tine-store/src/page_host/production_guard.rs#ProductionIo::check_path` | `crates/tine-store/src/page_host/production_guard_tests.rs#l8_01_actual_io_rechecks_after_admission_and_after_temp` | All five; C2 and transaction schedules pending M3 | guarded-untested |
| A-ROOT-BOUND | Application graph writes check the directory bound at open; Tine never creates the graph root or an ancestor | `disk: int -> int` | `crates/tine-store/src/root_binding.rs#RootBinding::check`<br>`crates/tine-store/src/directory_durability.rs#DirectoryCreation::new` | `crates/tine-store/src/page_host/production_guard_tests.rs#l9_replaced_root_refuses_creating_save_and_keeps_input`<br>`crates/tine-store/src/directory_durability.rs#l9_directory_creation_never_recreates_a_vanished_bound`<br>`crates/tine-store/src/restore.rs#l9_restore_refuses_a_replaced_root_before_reserving_recovery`<br>`crates/tine-store/src/page_host/native_conformance_tests.rs#l9_native_and_model_refuse_the_same_detached_root_phases`<br>`crates/tine-store/src/page_host/binding_root_tests.rs#l9_input_typed_after_root_moves_reaches_durable_custody`<br>`crates/tine-store/src/root_binding.rs#l9_root_binding_holds_no_directory_descriptor` | Linux/macOS/iOS stat identity with birth time where supplied; no held directory fd. Inode reuse after delete/recreate at the same path (with reused/unavailable birth time) remains a window. Windows file id runtime/cost is unverified. Android uses vanish-only until FUSE identity is device-verified, so replacement is not detected there. ModelFs epochs/native phase replay and real post-detachment admission cover Linux; accepted final check-to-OS race remains. Developer export and git are outside this guard. | guarded-untested |
| A-DRAFT-NAMESPACE | Relaunch finds pre-crash drafts after a root path change | `action launch` | `src-tauri/src/root_identity.rs#adopt_namespace` | `src-tauri/src/root_identity.rs#relocation_is_idempotent_and_preserves_sealed_bytes`<br>`src-tauri/src/root_identity.rs#adoption_requires_an_empty_target_and_moves_the_whole_census` | Existing formats; validated iOS auto-relocation, explicit orphan recovery elsewhere. iOS device rebase remains unverified. | guarded-untested |
| A-ONE-HOST | One host owns a graph and its drafts | `alive: bool` | `src-tauri/src/owner/lease.rs#claim`<br>`src-tauri/src/state.rs#GraphRegistry::draft_namespace_held` | `src-tauri/src/root_identity.rs#old_graph_id_uses_the_same_owner_lock_as_the_host`<br>`src-tauri/src/host_retirement.rs#l9_retiring_source_refuses_draft_recovery`<br>`src-tauri/src/host_retirement.rs#l9_recovery_and_relocation_exclude_both_bound_and_retiring_namespaces` | Linux/macOS/Windows account locks; iOS/Android per-app sandbox cross-process exclusion. On all five targets registry/retirement entries exclude both draft namespaces, except the saved-stop recovery target. Lock errors retain diagnosed fail-open; recovery refuses without the current desktop lease. Windows runtime remains unverified. | guarded-untested |
| A-DIRSYNC | Directory sync settles names; withdraw trash creation lacks the sync; root/parent identity is unbound | `action dirSync` | `crates/tine-store/src/directory_durability.rs#create_dir_all_durable` | `crates/tine-store/tests/directory_durability_guard.rs#directory_sync_has_one_owner_and_no_discarded_result` | All five; withdraw L9-13 and C6 power fault pending | unguarded |
| A-ATOMIC-READ | Reads return a whole version; torn-base conflict safety is unverified | `disk: int -> int` | — | — | All five; in-place writer C7 pending | assumed-outside |
| A-DRAFT-DURABLE | App-data drafts are durable on a strong backing store | `drafts: int -> Draft` | `crates/tine-store/src/page_host/production.rs#ProductionIo::draft_sync` | `crates/tine-store/src/page_host/native_conformance_tests.rs#native_committed_scenarios_in_four_profiles` | All five; Windows private directory sync and tmpfs classification L9-15 remain gaps | guarded-untested |
| A-CASE-DISTINCT | Distinct page keys name distinct files | `pure val PAGES` | `crates/tine-store/src/no_replace.rs#move_file_noreplace` | — | Folding/no-replace exist; general case/Unicode alias profile C8 pending | guarded-untested |
| A-ATOMIC-REPLACE | Publication is atomic old-or-new under the platform recipe | `action rename` | `crates/tine-store/src/atomic_file.rs#rename_replace` | `crates/tine-store/tests/i02_crash_gaps.rs#transaction_kill_reopen_preserves_each_old_or_new_file`<br>`crates/tine-store/tests/i16_platform_arms.rs#host_publication_witnesses_cover_exactly_the_five_shipped_targets` | All five recipes pinned; weak filesystems retain weaker durability | guarded+tested |
| A-DELETE-ONE-MOVE | Delete moves once into same-volume trash without replacing a target | `action rename` | `crates/tine-store/src/no_replace.rs#move_file_noreplace` | `crates/tine-store/src/page_host/native_conformance_tests.rs#native_committed_scenarios_in_four_profiles` | All five; unsupported rename-noreplace fallback has accepted creator race; errno-class test pending | guarded+tested |
| A-TRASH-COUPLED | Page-host deletion and its trash payload survive power loss together | `trashStable: int -> Set[int]` | `crates/tine-store/src/page_host/production.rs#ProductionIo::trash_sync` | `crates/tine-store/src/page_host/conformance_tests.rs#all_s3_scenarios_in_four_profiles` | Page host ModelFs power cuts; native has no power cut; transactions/withdraw outside oracle | guarded+tested |
| A-NAME-SLOT | Creation checks another claimant of the same logical name | `val claimedCreates` | `crates/tine-store/src/page_host/production_guard.rs#ProductionIo::creating_owner` | `crates/tine-store/src/page_host/production_guard_tests.rs#l8_02_raw_creation_rules_survive_failure_and_new_input_revokes_them` | All five; plain creating-save model admission JA5/s3.6 pending | guarded-untested |
| A-EXT-WHOLE | Environment writes replace whole files | `action extWriteD` | — | — | All five; in-place writes excluded, see A-ATOMIC-READ | assumed-outside |
| A-PROCESS-CUT | Process/window may die between syscalls | `action crash` | `crates/tine-store/src/atomic_file.rs#PreparedWrite::with_hooks` | `crates/tine-store/tests/i02_crash_gaps.rs#transaction_kill_reopen_preserves_each_old_or_new_file` | Page-host phase cuts and selected subprocess journeys; not native power-loss refinement (G7) | guarded+tested |
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
- A-DIRSYNC follow-up: L9-13, durable withdraw trash directory creation.

## Mutation ledger

The A-MUTATION-CONTAINMENT follow-up records the M1 census findings; it is
part of the same follow-up work, not a production fix in M0/M1:

- `crates/tine-store/src/bin/export-block-raws.rs#main` overwrites
  `<supplied graph>/block-raws.json` directly. It has no containment or atomic
  save guard. This is a developer export tool, not the application save path.
- `transaction.rs#Transaction::fault_collision` and `Transaction::fault_twin`
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
its hash or proof claim. A-DIRSYNC's unrelated withdraw/power obligations remain.

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
presence remain reviewed evidence: no taint analysis, call graph, ordering
proof or dynamic primitive hook is claimed. HostIo helpers have `oracle=false`;
only actual mutating seam methods have `oracle=true`. The oracle still does
not cover transactions, restore, publish, purge or git (environment writes).
M2–M4 obligations beyond the L9 root epoch schedules remain pending.

L9 per edit adds zero bytes/files/transport. Root checks add metadata calls at
graph I/O boundaries (six checks for the ordinary measured native save), and
open-time namespace discovery is bounded by the device graph list. See receipt.
No directory fd pins a volume. Linux statx btime and Apple st_birthtime reduce
the inode-reuse window; delete/recreate with reused inode and indistinguishable
birth time can still evade identity comparison. Android is deliberately
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
