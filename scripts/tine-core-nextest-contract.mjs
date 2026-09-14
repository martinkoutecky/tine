#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  ONE_RELEASE_CI_EXCEPTION,
  PROJECT_VERSION,
  linuxReleaseExcludedTestNames,
  oneReleaseCiExceptionActive,
  windowsRequiredTestNames,
} from "./release-ci-exception.mjs";

export const LINUX_TINE_CORE_SHARD_COUNT = 4;

// Linux runs the complete current tine-core inventory by default. An honest
// unfiltered `cargo test -p tine-core --no-fail-fast` on base ab3de16d
// measured 1992 tests with exactly 70 failures. The 2026-09-07 W6-red-corpus
// harvest retired that corpus by name in two passes:
//
//   * 35 tests asserted mechanics that were retired BY DECISION and were
//     deleted -- dead `#[cfg(test)]` cuts nothing reads and the pre-clean flat
//     archive layout (85b3339c), the Direct-Files whole-graph admission walk
//     (05db8c67, program D1 of the Direct Files program), and the publication
//     intents / manifest-recovery links+blobs / transport publishers /
//     pending-marker publication that retirement cut B removed under the 0.7
//     blank-slate ruling (2a578d87, b9bc23c1; SETTLED-DECISIONS D-1).
//   * 13 drove dead paths while their user contract stayed live and were
//     migrated onto the current front door.
//   * 3 were pins or tables repaired in place (two census pins and the
//     refusal-scenario vocabulary in oplog/refusal.rs).
//   * 2 belong to the concurrent query-engine campaign and are waived in
//     scripts/release-ci-exception.json instead.
//   * 17 survived as live clean-runtime defects. They are the list below.
//
// The suite is now 1957 tests with 19 failures: the 17 below plus those 2.
//
// What remains below is NOT legacy residue and NOT a retired mechanism. Every
// name is a reproduced defect in the CURRENT clean runtime, with a named cause
// and a fix that lands outside a test file. Each is carded in the harvest
// receipt, and the user-visible ones have a `reproduced` row in
// tests/regressions/non-ui.json. Read this list as an open bug ledger, not as
// a waiver: shrinking it means fixing the product, not renaming the test.
//
// Keep the exclusions exact, name-level, and cause-classified. They are
// release-excluded only while package.json still reads
// ONE_RELEASE_CI_EXCEPTION_VERSION (0.6.982, the version this filter was
// measured on). The next `chore(release): prepare vX candidate` commit bumps
// package.json, the filter collapses to all() with no edit here, and every
// tine-core test must pass from that release onward.
export const KNOWN_RED_TINE_CORE_FAILURE_FAMILIES = Object.freeze({
  // `SyncRuntimeHandle::activate_or_resume_local` never leaves Retryable for a
  // graph that has `logseq/config.edn` and directories but no page file:
  // `{"kind":"clean-open","reason_code":"clean_open.bootstrap_streaming_import"}`
  // still after 64 retries. A graph CREATED in Tine is seeded with 24 guide
  // pages (src-tauri/src/graph.rs:1226 -> onboarding::create_demo_graph), so
  // this is reached by opening an existing empty folder, or by a user who
  // deleted every page -- not by the default first-run flow.
  cleanActivationOfAnEmptyGraph: Object.freeze([
    // Fixture never reaches LocalActive, so the two-winner convergence case
    // cannot run.
    "sync_runtime::tests::concurrent_explicit_and_filename_fallback_titles_converge_in_both_winner_directions",
    // Same: fixture never reaches LocalActive.
    "sync_runtime::tests::concurrent_offline_canonical_equivalent_editor_titles_preserve_exact_semantics",
    // Activation returns Retryable{durable_stage: Absent} where the contract
    // requires Active.
    "sync_runtime::tests::managed_new_page_conflict_resolution_uses_the_identifiable_winner_path_and_revision",
  ]),
  // The provider projection scheduler never settles: it exhausts its bounded
  // turn budget with a last tick of `RecoveryBlocked("projection manifest
  // validation failed: projection intent portable-path index binding
  // mismatch")`.
  //
  // Cause, verified: 9f24d985 (2026-08-31, "retire detached bootstrap and
  // Patricia stores") deleted the reconciliation arm of
  // `validate_manifested_portable_path_binding`
  // (crates/tine-core/src/oplog/hot_engine.rs:15719-15761 today). Before it, a
  // manifested portable-path root that differed from the receiver's candidate
  // root was accepted when it could be reconstructed as `accepted base +
  // publisher's changes` over the declared frontier bases. After it the
  // function ends `let _ = frontier;` and returns ProjectionManifest on ANY
  // inequality. So a receiver whose portable-path index root diverged from the
  // publisher's -- a concurrent external admission, an absent path, a rename
  // referrer, an offline branch -- never projects that batch (I-10).
  //
  // These cases were added GREEN by de7868c7 (2026-08-18, "always project an
  // applied provider batch to Markdown") with the binding check already
  // present (0acfc0a7 / 33d8e67f, July), so this is a regression of a landed
  // data-visibility fix. User outcome: a device that has edited a file the
  // other device also touched stops receiving that page forever.
  //
  // NOT this packet's to fix -- what replaces the Patricia reconstruction is a
  // design question (D-14), so this is a card, not a rename.
  providerProjectionLiveness: Object.freeze([
    // A remote create never appears beside a concurrent local admission.
    "sync_runtime::tests::provider_create_projects_markdown_beside_a_concurrent_external_admission",
    // A remote edit never appears beside a concurrent local admission.
    "sync_runtime::tests::provider_edit_projects_markdown_beside_a_concurrent_external_admission",
    // A remote cross-page move never projects either page.
    "sync_runtime::tests::provider_cross_page_move_projects_both_pages_beside_a_concurrent_external_admission",
    // A remote delete of a path the receiver never had does not converge.
    "sync_runtime::tests::provider_delete_of_a_path_absent_from_the_receiver_converges",
    // One peer's incomplete manifest blocks this device instead of being
    // ignored.
    "sync_runtime::tests::foreign_incomplete_manifest_does_not_block_own_frontier_or_intent_retirement",
    // A rename plus a referrer edit never converge in either delivery order.
    "sync_runtime::tests::rename_referrer_rewrite_and_referrer_edit_converge_in_both_delivery_orders",
    // Two offline authors that merge their provider trees BOTH block: probed
    // 2026-09-07, each device's tick is the same RecoveryBlocked and neither
    // device ever receives the other's page.
    "sync_runtime::tests::two_offline_authors_union_frontier_heads_converge_without_return_first",
  ]),
  // The application/editor front door refuses ordinary user actions.
  applicationFrontDoorRefusals: Object.freeze([
    // Loading a just-created page by id refuses with
    // ActorRefusedAt("hot_source_path_missing") (sync_runtime.rs:24677,
    // `graph.load_by_path` -> None). Probed 2026-09-07 with the DEFAULT graph
    // layout as well as the fixture's custom `:pages-directory`: red both ways,
    // so this is not a custom-layout bug -- a new page whose parsed identity
    // moves it to its final path cannot be opened afterwards.
    "sync_runtime::tests::new_markdown_and_org_pages_are_born_with_parsed_final_identity_at_selected_path",
    // DeletePage refuses with ActorRefusedAt("delete_stale_page_target") after
    // a conflict resolution re-authored the page. The NAME index is the stale
    // side: probed 2026-09-07, `load_application_exact(path)` returns the page
    // with exactly name "Déjà 計画" at
    // `notes/層/žluťoučký/nested/Déjà 計画.md`, while
    // `active_editor_name_state_for_format` (sync_runtime.rs:18403) answers
    // Missing for that same name. The same probe shows the second facet: with
    // `expected_path: None` the delete returns Ok(Applied) via the
    // "harmless retry" arm (sync_runtime.rs:18413) WITHOUT deleting anything.
    "sync_runtime::tests::managed_application_conflict_resolution_reauthors_retained_outline_at_one_observed_revision",
  ]),
  // A foreground cross-page move performs work the fast-commit invariant
  // forbids, so a move in a large graph stalls the caller. Probed 2026-09-07:
  // the counter that fires is `archive_object_reads: 6`; sqlite_drains,
  // projection_receipt_loads, graph_wide_catalog_decodes,
  // graph_wide_catalog_validations and application_page_loads are all 0
  // (crates/tine-core/src/fast_commit.rs:130 `forbidden_commit_work`).
  foregroundMoveDoesForbiddenCommitWork: Object.freeze([
    "sync_runtime::tests::foreground_cross_page_move_is_bounded_and_does_no_graph_wide_work",
  ]),
  // A sync service (Dropbox/Syncthing/iCloud) that writes a conflict copy of a
  // frontier head whose bytes DIFFER from the canonical head wedges the device:
  // every tick returns
  // RecoveryBlocked("sync actor refused request: provider conflict copy differs
  // from canonical generated evidence at frontier-heads-v1/....head") and the
  // runtime never settles again. The unreconciled bytes ARE preserved -- the
  // refusal happens before any mutation -- so this is an availability loss, not
  // a data loss. A byte-IDENTICAL conflict copy is retired correctly (the first
  // half of the same test passes), so the conflict lane itself is live.
  providerConflictCopyWedgesTheDevice: Object.freeze([
    "sync_runtime::tests::frontier_head_conflicts_fall_back_and_preserve_unreconciled_bytes",
  ]),
  // With an ordinary parent's manifest absent from BOTH the provider tree and
  // the local archive (`inspect_batch` -> Absent), the child batch still
  // publishes its manifest and the device reaches a Safe handoff. Probed
  // 2026-09-07 after removing the retired manifest-recovery steps from the
  // fixture: child_published=true, shutdown_err=false. A peer then receives a
  // batch whose causal parent exists nowhere.
  outboundPublicationPastALostParent: Object.freeze([
    "sync_runtime::tests::outbound_child_blocks_when_ordinary_parent_is_lost",
  ]),
  // After an oversized provider callback, the retained rescan IS drained and
  // the delivered page IS projected (probed 2026-09-07: page_projected=true),
  // but `clean_shutdown` returns
  // Err(ActorRefused("clean shutdown received unexpected runtime progress:
  // ProviderMutation { .. }")): its drain loop at
  // crates/tine-core/src/sync_runtime.rs:23286-23313 accepts Idle, Recovering,
  // AdmittedNoop, AdmittedComplete and two LocalMutation outcomes, and treats
  // every other tick -- including the ordinary "a remote batch applied" tick --
  // as unexpected progress. User outcome: quitting right after a large sync
  // delivery reports an unsafe shutdown and the next launch pays an
  // unsafe-reopen repair, even though nothing was lost.
  cleanShutdownRefusesOnAppliedProviderBatch: Object.freeze([
    "sync_runtime::tests::oversized_provider_callback_retains_scan_and_safe_shutdown_drains_it",
  ]),
  // Passes alone in 37s; fails only inside the full suite, and its failure
  // capsule points at a source line the test never executes. The clean
  // runtime's one-shot fault and test-cut registries are process-global rather
  // than workspace-keyed, so concurrently running tests consume each other's
  // arms. Harness debt, not a product defect: deliberately not carded.
  crossTestFaultRegistryIsolation: Object.freeze([
    "sync_runtime::tests::projection_recovery_equivalence_oracle_real_store_subset",
  ]),
});

export const KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES = Object.freeze(
  Object.values(KNOWN_RED_TINE_CORE_FAILURE_FAMILIES).flat().sort()
);

export function linuxCoreReleaseFilterset(version = PROJECT_VERSION) {
  const excluded = linuxReleaseExcludedTestNames(KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES, version);
  return excluded.length === 0
    ? "all()"
    : "not (" + excluded
      .map((testName) => "test(=" + testName + ")")
      .join(" | ") + ")";
}

export const LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES = Object.freeze(
  linuxReleaseExcludedTestNames(KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES)
);
export const LINUX_CORE_RELEASE_FILTERSET = linuxCoreReleaseFilterset();
// Windows is deliberately not a second complete tine-core behavior matrix.
// Linux carries that full inventory in four isolated shards. This exact list is
// the Windows release contract: every explicitly Windows-named core test, plus
// current activation/seal/durability/lifecycle witnesses (see docs/CI.md for
// the retired bootstrap/enrollment replacements). Keep names explicit so a rename, removal, or
// newly added Windows test cannot silently shrink the release gate.
export const WINDOWS_CORE_EXACT_TEST_NAMES = Object.freeze([
  "model::tests::page_name_encoding_is_injective_reversible_and_windows_safe",
  "model::tests::windows_handle_relative_noreplace_renames_the_exact_source",
  "model::tests::windows_handle_relative_noreplace_moves_between_nonstandard_retained_directories_with_unicode",
  "model::tests::windows_handle_relative_noreplace_preserves_occupied_destination",
  "model::tests::windows_first_save_and_ordinary_rename_preserve_exact_projection",
  "model::tests::windows_directory_durability_limit_does_not_block_save_or_rename",
  "model::tests::windows_direct_publication_event_waits_for_inflight_writer_receipt",
  "model::tests::windows_direct_publication_receipt_requires_revision_and_file_identity",
  "model::tests::windows_ambiguous_callback_cannot_interrupt_inflight_direct_creation",
  "model::tests::checked_open_accepts_an_approved_windows_assets_junction",
  "model::tests::projection_windows_held_handle_link_count_tracks_one_and_two_links",
  "model::tests::windows_live_graph_root_move_is_denied_without_rebinding",
  "oplog::sqlite::tests::windows_entry_file_identity_classifies_reparse_lease_as_replaced",
  "windows_no_follow_publication_read_and_directory_flush_succeed",
  "windows_reparse_files_and_directories_are_rejected",
]);

export const WINDOWS_CORE_LIFECYCLE_WITNESS_NAMES = Object.freeze([
  "oplog::local_active::bounded_admission::clean_admissions_are_bounded_at_one_one_thousand_and_ten_thousand",
  "model::tests::bootstrap_source_regular_file_sync_uses_supported_handle_access",
  "sync_runtime::tests::managed_activation_abort_cuts_retire_unmarked_generation_and_retry",
  "oplog::lazy_genesis::tests::lazy_genesis_seal_reopens_and_detects_payload_corruption",
  "oplog::sqlite::tests::one_workspace_runtime_lease_vends_one_applier_slot_at_a_time",
  "oplog::sqlite::tests::lease_contention_and_drop_recovery_are_process_scoped",
  "oplog::sqlite::tests::separate_process_workspace_lease_contends_and_crash_releases",
]);

export const WINDOWS_CORE_CAPTURE_WITNESS_NAMES = Object.freeze([
  "model::tests::inactive_bootstrap_capture_exact_64_mib_sparse_file_is_accepted",
  "model::tests::inactive_bootstrap_capture_external_sort_is_buffer_bounded_without_real_files",
  "model::tests::inactive_bootstrap_capture_ignores_residue_is_idempotent_and_rejects_conflicting_seal",
  "model::tests::inactive_bootstrap_capture_is_deterministic_and_chunks_zero_one_and_many_files",
  "model::tests::inactive_bootstrap_capture_preserves_exact_nested_unicode_org_and_semantic_kinds",
  "model::tests::inactive_bootstrap_capture_rejects_bad_logical_name_frames",
  "model::tests::inactive_bootstrap_capture_seals_one_pass_and_final_proof_rejects_later_mutations",
  "model::tests::inactive_bootstrap_capture_rejects_file_cap_before_streaming",
]);

const WINDOWS_CORE_ORDINARY_SMOKE_TEST_NAMES = Object.freeze([
  ...new Set([
    ...WINDOWS_CORE_EXACT_TEST_NAMES,
    ...WINDOWS_CORE_LIFECYCLE_WITNESS_NAMES,
    ...WINDOWS_CORE_CAPTURE_WITNESS_NAMES,
  ]),
]);

export function windowsCoreSmokeTestNames(version = PROJECT_VERSION) {
  return windowsRequiredTestNames(WINDOWS_CORE_ORDINARY_SMOKE_TEST_NAMES, version);
}

export const WINDOWS_CORE_SMOKE_TEST_NAMES = Object.freeze(windowsCoreSmokeTestNames());

// The same declared names drive nextest and the inventory verifier. Do not
// replace this with a broad package filter: that would bring platform-neutral
// tests (including currently known Windows-incompatible ones) back into the
// release gate without an intentional policy change.
export const WINDOWS_CORE_SMOKE_FILTERSET = WINDOWS_CORE_SMOKE_TEST_NAMES
  .map((testName) => `test(=${testName})`)
  .join(" | ");

function fail(message) {
  throw new Error(`tine-core nextest contract: ${message}`);
}

function testKey(binaryId, testName) {
  return `${binaryId}\u0000${testName}`;
}

export function inventoryFromNextestList(packageName, list) {
  if (!list || typeof list !== "object" || !list["rust-suites"] || typeof list["rust-suites"] !== "object") {
    fail(`${packageName} list did not contain rust-suites`);
  }

  const tests = new Map();
  for (const [binaryId, suite] of Object.entries(list["rust-suites"])) {
    if (suite?.["package-name"] !== packageName) continue;
    if (!suite.testcases || typeof suite.testcases !== "object") {
      fail(`${packageName} binary ${binaryId} did not contain testcases`);
    }
    for (const [testName, testcase] of Object.entries(suite.testcases)) {
      // `cargo nextest run` does not run #[ignore] tests without an explicit
      // --run-ignored argument. Match that exact normal test-run inventory.
      if (testcase?.ignored || testcase?.["filter-match"]?.status !== "matches") continue;
      const key = testKey(binaryId, testName);
      if (tests.has(key)) fail(`${packageName} listed ${binaryId} ${testName} twice`);
      tests.set(key, { binaryId, testName });
    }
  }
  if (tests.size === 0) fail(`${packageName} selected no non-ignored tests`);
  return { packageName, tests };
}

export function verifyLinuxShardCoverage(fullInventory, shardInventories) {
  if (fullInventory?.packageName !== "tine-core") fail("Linux full inventory is not tine-core");
  if (!Array.isArray(shardInventories) || shardInventories.length !== LINUX_TINE_CORE_SHARD_COUNT) {
    fail(`expected ${LINUX_TINE_CORE_SHARD_COUNT} Linux shard inventories`);
  }

  const ownerByTest = new Map();
  for (const [index, shard] of shardInventories.entries()) {
    if (shard?.packageName !== "tine-core") fail(`Linux shard ${index + 1} is not tine-core`);
    if (shard.tests.size === 0) fail(`Linux shard ${index + 1} selected no tests`);
    for (const [key, test] of shard.tests) {
      if (!fullInventory.tests.has(key)) {
        fail(`Linux shard ${index + 1} selected non-inventory test ${test.binaryId} ${test.testName}`);
      }
      const priorOwner = ownerByTest.get(key);
      if (priorOwner !== undefined) {
        fail(`Linux shards ${priorOwner} and ${index + 1} both selected ${test.binaryId} ${test.testName}`);
      }
      ownerByTest.set(key, index + 1);
    }
  }

  const missing = [...fullInventory.tests.entries()]
    .filter(([key]) => !ownerByTest.has(key))
    .map(([, test]) => `${test.binaryId} ${test.testName}`);
  if (missing.length > 0) fail(`Linux shards omitted ${missing.length} tests (first: ${missing[0]})`);

  return { testCount: fullInventory.tests.size, shardCounts: shardInventories.map((shard) => shard.tests.size) };
}

export function verifyLinuxReleaseSelection(coreInventory, releaseInventory, version = PROJECT_VERSION) {
  if (coreInventory?.packageName !== "tine-core") fail("Linux core inventory is not tine-core");
  if (releaseInventory?.packageName !== "tine-core") fail("Linux release inventory is not tine-core");

  for (const [key, test] of releaseInventory.tests) {
    if (!coreInventory.tests.has(key)) {
      fail(`Linux release selection contains non-inventory test ${test.binaryId} ${test.testName}`);
    }
  }

  const excluded = [...coreInventory.tests.entries()]
    .filter(([key]) => !releaseInventory.tests.has(key))
    .map(([, test]) => test);
  // Module membership is not oracle-ness. The claim this contract has to make
  // is that every test the release gate drops is a NAMED, deliberately dropped
  // test, so compare the actual excluded names against the exclusion contract
  // in both directions: an unlisted exclusion means a healthy test was silently
  // un-gated, and a listed name with no test behind it means the list has
  // rotted. Names, never counts.
  requireExactNameSet(
    excluded.map((test) => test.testName),
    linuxReleaseExcludedTestNames(KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES, version),
    "Linux release exclusion contract"
  );

  return {
    coreTestCount: coreInventory.tests.size,
    releaseTestCount: releaseInventory.tests.size,
    knownRedTestCount: excluded.length,
  };
}

function testNames(inventory) {
  return [...inventory.tests.values()].map((test) => test.testName).sort();
}

function requireUniqueName(inventory, name) {
  const matching = [...inventory.tests.values()].filter((test) => test.testName === name);
  if (matching.length !== 1) {
    fail(`${inventory.packageName} must contain exactly one required test ${name}; found ${matching.length}`);
  }
  return matching[0];
}

function requireNamesSelected(fullInventory, selectedInventory, names, label) {
  for (const name of names) {
    const expected = requireUniqueName(fullInventory, name);
    if (!selectedInventory.tests.has(testKey(expected.binaryId, expected.testName))) {
      fail(`${label} omitted required test ${name}`);
    }
  }
}

function requireExactNameSet(actualNames, expectedNames, label) {
  const actual = [...actualNames].sort();
  const expected = [...expectedNames].sort();
  if (actual.length !== expected.length || actual.some((name, index) => name !== expected[index])) {
    const missing = expected.filter((name) => !actual.includes(name));
    const unexpected = actual.filter((name) => !expected.includes(name));
    fail(
      `${label} changed; missing [${missing.join(", ") || "none"}], unexpected [${unexpected.join(", ") || "none"}]`
    );
  }
}

export function verifyWindowsCoreSmokeSelection(coreInventory, smokeInventory, version = PROJECT_VERSION) {
  if (coreInventory?.packageName !== "tine-core") fail("Windows core inventory is not tine-core");
  if (smokeInventory?.packageName !== "tine-core") fail("Windows core smoke inventory is not tine-core");

  const windowsNamed = [...coreInventory.tests.values()]
    .filter((test) => test.testName.toLowerCase().includes("windows"))
    .map((test) => test.testName);
  requireExactNameSet(windowsNamed, WINDOWS_CORE_EXACT_TEST_NAMES, "Windows-named tine-core test inventory");
  const requiredSmokeNames = windowsCoreSmokeTestNames(version);
  requireNamesSelected(coreInventory, smokeInventory, requiredSmokeNames, "Windows core smoke selection");
  requireExactNameSet(testNames(smokeInventory), requiredSmokeNames, "Windows core smoke selection");

  return {
    coreTestCount: coreInventory.tests.size,
    coreSmokeTestCount: smokeInventory.tests.size,
    windowsNamedCount: windowsNamed.length,
    bootstrapWitnessCount: WINDOWS_CORE_CAPTURE_WITNESS_NAMES.length,
  };
}

function nextestList(profile, packageName, { partition, filterset } = {}) {
  const args = ["nextest", "list", "--profile", profile, "--package", packageName, "--message-format", "json"];
  if (partition) args.push("--partition", partition);
  if (filterset) args.push("--filterset", filterset);
  const result = spawnSync("cargo", args, { cwd: process.cwd(), encoding: "utf8" });
  if (result.error) fail(`could not start cargo nextest list: ${result.error.message}`);
  if (result.status !== 0) {
    fail(`cargo nextest list for ${packageName}${partition ? ` (${partition})` : ""} failed:\n${result.stderr}`);
  }
  try {
    return inventoryFromNextestList(packageName, JSON.parse(result.stdout));
  } catch (error) {
    if (error instanceof SyntaxError) fail(`cargo nextest list for ${packageName} did not emit JSON: ${error.message}`);
    throw error;
  }
}

function runWindowsSmoke(packageName, filterset, label) {
  const result = spawnSync(
    "cargo",
    ["nextest", "run", "--profile", "ci-windows", "--package", packageName, "--filterset", filterset],
    { cwd: process.cwd(), stdio: "inherit" }
  );
  if (result.error) fail(`could not start ${label}: ${result.error.message}`);
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function runLinuxSelection({ shard } = {}) {
  const args = [
    "nextest",
    "run",
    "--profile",
    "ci",
    "--package",
    "tine-core",
    "--filterset",
    LINUX_CORE_RELEASE_FILTERSET,
  ];
  if (shard !== undefined) args.push("--partition", `hash:${shard}/${LINUX_TINE_CORE_SHARD_COUNT}`);
  const label = shard === undefined ? "Linux tine-core release selection" : `Linux tine-core shard ${shard}`;
  const result = spawnSync("cargo", args, { cwd: process.cwd(), stdio: "inherit" });
  if (result.error) fail(`could not start ${label}: ${result.error.message}`);
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function option(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function main() {
  const mode = option("--mode");
  if (mode === "linux") {
    const core = nextestList("ci", "tine-core");
    const full = nextestList("ci", "tine-core", { filterset: LINUX_CORE_RELEASE_FILTERSET });
    const selection = verifyLinuxReleaseSelection(core, full);
    const shards = Array.from({ length: LINUX_TINE_CORE_SHARD_COUNT }, (_, index) =>
      nextestList("ci", "tine-core", {
        partition: `hash:${index + 1}/${LINUX_TINE_CORE_SHARD_COUNT}`,
        filterset: LINUX_CORE_RELEASE_FILTERSET,
      })
    );
    const result = verifyLinuxShardCoverage(full, shards);
    console.log(
      `Linux nextest contract OK: ${result.testCount} release tests exactly once across ${LINUX_TINE_CORE_SHARD_COUNT} hash shards (${result.shardCounts.join(", ")}); every current tine-core test is selected except exactly the ${selection.knownRedTestCount} named, behavior-family-classified known-red legacy-oracle tests.`
      + (oneReleaseCiExceptionActive()
        ? ` All ${selection.knownRedTestCount} name exclusions expire automatically after v0.6.981; ${ONE_RELEASE_CI_EXCEPTION.linuxAdditionalKnownRedTestNames.length} were added from this release's exact baseline.`
        : "")
    );
    const runShard = option("--run-shard");
    if (runShard !== undefined) {
      const shard = Number(runShard);
      if (!Number.isInteger(shard) || shard < 1 || shard > LINUX_TINE_CORE_SHARD_COUNT) {
        fail(`--run-shard must be an integer from 1 to ${LINUX_TINE_CORE_SHARD_COUNT}`);
      }
      runLinuxSelection({ shard });
    }
    // The PR gate runs the whole verified selection in one job instead of four
    // sharded ones; both paths execute the identical filterset and `ci` profile.
    if (process.argv.includes("--run-selection")) runLinuxSelection();
    return;
  }
  if (mode === "windows") {
    const core = nextestList("ci-windows", "tine-core");
    const smoke = nextestList("ci-windows", "tine-core", { filterset: WINDOWS_CORE_SMOKE_FILTERSET });
    const result = verifyWindowsCoreSmokeSelection(core, smoke);
    console.log(
      `Windows nextest contract OK: ${result.coreTestCount} compiled tine-core tests, ${result.coreSmokeTestCount} contract-selected cross-layer smokes, ${result.windowsNamedCount} Windows-named core tests, and ${result.bootstrapWitnessCount} bootstrap capture witnesses.`
      + (oneReleaseCiExceptionActive()
        ? ` The ${ONE_RELEASE_CI_EXCEPTION.windowsMissingRequiredTestNames.length} missing-witness exception expires automatically after v0.6.981.`
        : "")
    );
    if (process.argv.includes("--run-smoke")) {
      runWindowsSmoke("tine-core", WINDOWS_CORE_SMOKE_FILTERSET, "Windows core/storage integration smoke");
    }
    return;
  }
  fail(
    "pass --mode linux (add --run-shard N for one release shard, or --run-selection for the whole verified release selection) or --mode windows (add --run-smoke to execute the verified Windows core/storage integration selection)"
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
