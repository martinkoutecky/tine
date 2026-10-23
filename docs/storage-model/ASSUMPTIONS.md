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
| A-ROOT-BOUND | The root stays bound; no write recreates it or its ancestors | `disk: int -> int` | — | — | All five; L9-02, C1 pending | unguarded |
| A-DRAFT-NAMESPACE | Relaunch finds pre-crash drafts after a root path change | `action launch` | — | — | All five; L9-01, C5 pending | unguarded |
| A-ONE-HOST | One host owns a graph and its drafts | `alive: bool` | — | — | Linux/macOS arbitration exists; Windows/iOS/Android lack it (L9-12) | unguarded |
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

- A-ROOT-BOUND follow-up: L9-02, RootBinding and bounded creation.
- A-DRAFT-NAMESPACE follow-up: L9-01, stable draft/backup namespace.
- A-ONE-HOST follow-up: L9-12, arbitration on the other three platforms.
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

The existing containment checks on other graph rows still do not bind root
identity (A-ROOT-BOUND). `Graph::withdraw_file_to_conflict_if` checks containment
but creates its trash directory without a durable directory protocol
(A-DIRSYNC). Those distinct missing guards remain explicitly unguarded above.

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
M2–M4 environment faults and richer ModelFs are pending; none are added here.

Per-edit cost: zero bytes, files, syscalls and transport bytes; tests/docs only.
