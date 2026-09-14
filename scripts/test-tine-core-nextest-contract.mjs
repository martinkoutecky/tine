#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  LINUX_CORE_RELEASE_FILTERSET,
  LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES,
  LINUX_TINE_CORE_SHARD_COUNT,
  KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES,
  KNOWN_RED_TINE_CORE_FAILURE_FAMILIES,
  WINDOWS_CORE_CAPTURE_WITNESS_NAMES,
  WINDOWS_CORE_EXACT_TEST_NAMES,
  WINDOWS_CORE_LIFECYCLE_WITNESS_NAMES,
  WINDOWS_CORE_SMOKE_FILTERSET,
  WINDOWS_CORE_SMOKE_TEST_NAMES,
  inventoryFromNextestList,
  linuxCoreReleaseFilterset,
  verifyLinuxReleaseSelection,
  verifyLinuxShardCoverage,
  verifyWindowsCoreSmokeSelection,
  windowsCoreSmokeTestNames,
} from "./tine-core-nextest-contract.mjs";
import {
  ONE_RELEASE_CI_EXCEPTION,
  ONE_RELEASE_CI_EXCEPTION_VERSION,
  PROJECT_VERSION,
  classifyRetiredManagedV1Problems,
  linuxReleaseExcludedTestNames,
  oneReleaseCiExceptionActive,
  releaseE2eScenarioIsNonblocking,
  windowsRequiredTestNames,
} from "./release-ci-exception.mjs";

// Advance this with every re-baselining of the CI exception. It is the ratchet
// that forces the waiver to be re-measured and re-approved each release instead
// of quietly becoming permanent: the assertions below prove the exception is
// INACTIVE here, so a release that has not re-decided cannot inherit it.
const NEXT_RELEASE_VERSION = "0.6.983";

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function listedInventory(packageName, testNames) {
  return inventoryFromNextestList(packageName, {
    "rust-suites": {
      [packageName]: {
        "package-name": packageName,
        testcases: Object.fromEntries(testNames.map((testName) => [testName, {
          ignored: false,
          "filter-match": { status: "matches" },
        }])),
      },
    },
  });
}

const shardNames = ["alpha", "beta", "gamma", "delta"];
const fullCore = listedInventory("tine-core", shardNames);
const shards = shardNames.map((name) => listedInventory("tine-core", [name]));
assert.deepEqual(
  verifyLinuxShardCoverage(fullCore, shards),
  { testCount: 4, shardCounts: [1, 1, 1, 1] }
);
assert.throws(
  () => verifyLinuxShardCoverage(fullCore, [shards[0], shards[1], shards[2], listedInventory("tine-core", [])]),
  /selected no non-ignored tests/
);

const releaseSelectedNames = [
  "model::tests::ordinary_semantic_contract",
  "sync_runtime::tests::current_clean_runtime_contract",
  "sync_runtime::tests::new_production_test_is_selected_automatically",
];
const coreWithKnownRedOracle = listedInventory("tine-core", [
  ...releaseSelectedNames,
  ...LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES,
]);
const releaseWithoutKnownRedOracle = listedInventory("tine-core", releaseSelectedNames);
assert.deepEqual(
  verifyLinuxReleaseSelection(coreWithKnownRedOracle, releaseWithoutKnownRedOracle),
  {
    coreTestCount: releaseSelectedNames.length + LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES.length,
    releaseTestCount: releaseSelectedNames.length,
    knownRedTestCount: LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES.length,
  }
);
assert.throws(
  () => verifyLinuxReleaseSelection(
    coreWithKnownRedOracle,
    listedInventory("tine-core", releaseSelectedNames.slice(0, 2))
  ),
  /Linux release exclusion contract changed.*new_production_test_is_selected_automatically/
);

// And a listed exclusion whose test no longer exists must fail too, so the list
// cannot rot through renames or deletions.
if (LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES.length > 0) {
  const staleOracleName = LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES[0];
  const coreWithoutOneOracleTest = listedInventory("tine-core", [
    ...releaseSelectedNames,
    ...LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES.filter((name) => name !== staleOracleName),
  ]);
  assert.throws(
    () => verifyLinuxReleaseSelection(coreWithoutOneOracleTest, releaseWithoutKnownRedOracle),
    new RegExp(`Linux release exclusion contract changed; missing \\[${staleOracleName}\\]`)
  );

}

// The allow-by-default filter must not permit a non-oracle omission, whether
// the omitted test is in sync_runtime or another module.
assert.throws(
  () => verifyLinuxReleaseSelection(
    coreWithKnownRedOracle,
    listedInventory("tine-core", releaseSelectedNames.filter((name) => !name.startsWith("model::tests::")))
  ),
  /Linux release exclusion contract changed.*model::tests::ordinary_semantic_contract/
);
const familyNames = Object.values(KNOWN_RED_TINE_CORE_FAILURE_FAMILIES).flat();
assert.deepEqual([...familyNames].sort(), KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES);
assert.deepEqual(Object.keys(KNOWN_RED_TINE_CORE_FAILURE_FAMILIES), [
  "cleanActivationOfAnEmptyGraph",
  "providerProjectionLiveness",
  "applicationFrontDoorRefusals",
  "foregroundMoveDoesForbiddenCommitWork",
  "providerConflictCopyWedgesTheDevice",
  "outboundPublicationPastALostParent",
  "cleanShutdownRefusesOnAppliedProviderBatch",
  "crossTestFaultRegistryIsolation",
]);
for (const names of Object.values(KNOWN_RED_TINE_CORE_FAILURE_FAMILIES)) {
  assert.ok(names.length > 0);
}
// Round 2 of the W6-red-corpus harvest re-bucketed the `model::tests::`
// admission family: its subject -- the Direct-save whole-graph, parser-owned
// admission walk -- was cut on purpose by 05db8c67 (program D1 of the Direct
// Files program), so those tests were deleted rather than carded. The residual
// corpus is therefore `sync_runtime::tests::` only again, and every remaining
// name is a live clean-runtime defect.
for (const name of KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES) {
  assert.match(name, /^sync_runtime::tests::/);
}
assert.equal(
  new Set(KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES).size,
  KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES.length
);

const exceptionActive = PROJECT_VERSION === ONE_RELEASE_CI_EXCEPTION_VERSION;
assert.equal(oneReleaseCiExceptionActive(), exceptionActive);
assert.equal(oneReleaseCiExceptionActive(ONE_RELEASE_CI_EXCEPTION_VERSION), true);
assert.equal(oneReleaseCiExceptionActive(NEXT_RELEASE_VERSION), false);
assert.deepEqual(
  ONE_RELEASE_CI_EXCEPTION.releaseE2eNonblockingScenarioKeys,
  ["linux-release:managed-journal-feed"]
);
assert.equal(releaseE2eScenarioIsNonblocking("linux-release", "managed-journal-feed"), exceptionActive);
assert.equal(releaseE2eScenarioIsNonblocking("linux-release", "managed-journal-feed", NEXT_RELEASE_VERSION), false);
assert.equal(releaseE2eScenarioIsNonblocking("linux-release", "some-other-scenario"), false);
assert.deepEqual(
  LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES,
  linuxReleaseExcludedTestNames(KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES)
);
assert.equal(
  LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES.length,
  exceptionActive ? KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES.length
    + ONE_RELEASE_CI_EXCEPTION.linuxAdditionalKnownRedTestNames.length : 0
);
assert.deepEqual(
  linuxReleaseExcludedTestNames(KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES, NEXT_RELEASE_VERSION),
  []
);
assert.equal(linuxCoreReleaseFilterset(NEXT_RELEASE_VERSION), "all()");
if (!exceptionActive) {
  assert.equal(LINUX_CORE_RELEASE_FILTERSET, "all()");
  assert.deepEqual(LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES, []);
  assert.equal(releaseE2eScenarioIsNonblocking("linux-release", "managed-journal-feed"), false);
}
assert.deepEqual(
  linuxReleaseExcludedTestNames(KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES, ONE_RELEASE_CI_EXCEPTION_VERSION),
  [...new Set([...KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES, ...ONE_RELEASE_CI_EXCEPTION.linuxAdditionalKnownRedTestNames])].sort()
);
const releaseWaivedOnlyRed = ONE_RELEASE_CI_EXCEPTION.linuxAdditionalKnownRedTestNames[0];
assert.match(linuxCoreReleaseFilterset(ONE_RELEASE_CI_EXCEPTION_VERSION), new RegExp(`test\\(=${escapeRegExp(releaseWaivedOnlyRed)}\\)`));
assert.doesNotMatch(
  linuxCoreReleaseFilterset(NEXT_RELEASE_VERSION),
  new RegExp(`test\\(=${escapeRegExp(releaseWaivedOnlyRed)}\\)`)
);
assert.throws(
  () => verifyLinuxReleaseSelection(
    listedInventory("tine-core", [
      ...releaseSelectedNames,
      ...KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES,
      releaseWaivedOnlyRed,
    ]),
    listedInventory("tine-core", [
      ...releaseSelectedNames,
      ...KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES,
    ]),
    NEXT_RELEASE_VERSION
  ),
  new RegExp(`Linux release exclusion contract changed.*${releaseWaivedOnlyRed}`)
);
assert.deepEqual(
  verifyLinuxReleaseSelection(coreWithKnownRedOracle, coreWithKnownRedOracle, NEXT_RELEASE_VERSION),
  {
    coreTestCount: releaseSelectedNames.length + LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES.length,
    releaseTestCount: releaseSelectedNames.length + LINUX_CORE_RELEASE_EXCLUDED_TEST_NAMES.length,
    knownRedTestCount: 0,
  }
);

assert.match(
  linuxCoreReleaseFilterset(ONE_RELEASE_CI_EXCEPTION_VERSION),
  /not \(test\(=/
);
assert.match(linuxCoreReleaseFilterset(ONE_RELEASE_CI_EXCEPTION_VERSION), /test\(=sync_runtime::tests::/);
assert.doesNotMatch(LINUX_CORE_RELEASE_FILTERSET, /not test\(\/sync_runtime::tests::\/\)/);
assert.throws(
  () => verifyLinuxShardCoverage(fullCore, [shards[0], shards[1], shards[2], shards[2]]),
  /both selected tine-core gamma/
);

const coreWindowsTests = [
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
];
assert.deepEqual(WINDOWS_CORE_EXACT_TEST_NAMES, coreWindowsTests);

const coreLifecycleWitnesses = [
  "oplog::local_active::bounded_admission::clean_admissions_are_bounded_at_one_one_thousand_and_ten_thousand",
  "model::tests::bootstrap_source_regular_file_sync_uses_supported_handle_access",
  "sync_runtime::tests::managed_activation_abort_cuts_retire_unmarked_generation_and_retry",
  "oplog::lazy_genesis::tests::lazy_genesis_seal_reopens_and_detects_payload_corruption",
  "oplog::sqlite::tests::one_workspace_runtime_lease_vends_one_applier_slot_at_a_time",
  "oplog::sqlite::tests::lease_contention_and_drop_recovery_are_process_scoped",
  "oplog::sqlite::tests::separate_process_workspace_lease_contends_and_crash_releases",
];
assert.deepEqual(WINDOWS_CORE_LIFECYCLE_WITNESS_NAMES, coreLifecycleWitnesses);

const coreCaptureWitnesses = [
  "model::tests::inactive_bootstrap_capture_exact_64_mib_sparse_file_is_accepted",
  "model::tests::inactive_bootstrap_capture_external_sort_is_buffer_bounded_without_real_files",
  "model::tests::inactive_bootstrap_capture_ignores_residue_is_idempotent_and_rejects_conflicting_seal",
  "model::tests::inactive_bootstrap_capture_is_deterministic_and_chunks_zero_one_and_many_files",
  "model::tests::inactive_bootstrap_capture_preserves_exact_nested_unicode_org_and_semantic_kinds",
  "model::tests::inactive_bootstrap_capture_rejects_bad_logical_name_frames",
  "model::tests::inactive_bootstrap_capture_seals_one_pass_and_final_proof_rejects_later_mutations",
  "model::tests::inactive_bootstrap_capture_rejects_file_cap_before_streaming",
];
assert.deepEqual(WINDOWS_CORE_CAPTURE_WITNESS_NAMES, coreCaptureWitnesses);

const ordinaryCoreSmokeTests = [...new Set([...coreWindowsTests, ...coreLifecycleWitnesses, ...coreCaptureWitnesses])];
const currentCoreSmokeTests = windowsCoreSmokeTestNames();
assert.deepEqual(WINDOWS_CORE_SMOKE_TEST_NAMES, currentCoreSmokeTests);
assert.deepEqual(
  currentCoreSmokeTests,
  windowsRequiredTestNames(ordinaryCoreSmokeTests)
);
for (const missingWindowsWitness of ONE_RELEASE_CI_EXCEPTION.windowsMissingRequiredTestNames) {
  assert.equal(currentCoreSmokeTests.includes(missingWindowsWitness), false);
}
assert.deepEqual(windowsCoreSmokeTestNames(NEXT_RELEASE_VERSION), ordinaryCoreSmokeTests);
assert.deepEqual(
  verifyWindowsCoreSmokeSelection(
    listedInventory("tine-core", [...currentCoreSmokeTests, "unselected_platform_neutral_test"]),
    listedInventory("tine-core", currentCoreSmokeTests)
  ),
  {
    coreTestCount: currentCoreSmokeTests.length + 1,
    coreSmokeTestCount: currentCoreSmokeTests.length,
    windowsNamedCount: 15,
    bootstrapWitnessCount: 8,
  }
);
assert.throws(
  () => verifyWindowsCoreSmokeSelection(
    listedInventory("tine-core", currentCoreSmokeTests.filter((name) => !name.includes("windows_live_graph"))),
    listedInventory("tine-core", currentCoreSmokeTests.filter((name) => !name.includes("windows_live_graph")))
  ),
  /Windows-named tine-core test inventory changed/
);
assert.throws(
  () => verifyWindowsCoreSmokeSelection(
    listedInventory("tine-core", currentCoreSmokeTests),
    listedInventory("tine-core", currentCoreSmokeTests.filter((name) => !name.includes("inactive_bootstrap_capture_rejects_file_cap")))
  ),
  /Windows core smoke selection omitted required test/
);
for (const missingWindowsWitness of coreLifecycleWitnesses) {
  const nextReleaseWithoutOneWitness = ordinaryCoreSmokeTests.filter((name) => name !== missingWindowsWitness);
  assert.throws(
    () => verifyWindowsCoreSmokeSelection(
      listedInventory("tine-core", nextReleaseWithoutOneWitness),
      listedInventory("tine-core", nextReleaseWithoutOneWitness),
      NEXT_RELEASE_VERSION
    ),
    new RegExp(`must contain exactly one required test ${missingWindowsWitness}`)
  );
}
assert.deepEqual(
  verifyWindowsCoreSmokeSelection(
    listedInventory("tine-core", [...ordinaryCoreSmokeTests, "unselected_platform_neutral_test"]),
    listedInventory("tine-core", ordinaryCoreSmokeTests),
    NEXT_RELEASE_VERSION
  ),
  {
    coreTestCount: ordinaryCoreSmokeTests.length + 1,
    coreSmokeTestCount: ordinaryCoreSmokeTests.length,
    windowsNamedCount: 15,
    bootstrapWitnessCount: 8,
  }
);

const retiredWaivedProblem = ONE_RELEASE_CI_EXCEPTION.retiredManagedV1AllowedProblems[0];
assert.deepEqual(
  classifyRetiredManagedV1Problems([retiredWaivedProblem], ONE_RELEASE_CI_EXCEPTION_VERSION),
  { allowed: [retiredWaivedProblem], unexpected: [] }
);
assert.deepEqual(
  classifyRetiredManagedV1Problems([retiredWaivedProblem], NEXT_RELEASE_VERSION),
  { allowed: [], unexpected: [retiredWaivedProblem] }
);
assert.deepEqual(
  classifyRetiredManagedV1Problems([retiredWaivedProblem, "new retired-v1 problem"], ONE_RELEASE_CI_EXCEPTION_VERSION),
  { allowed: [retiredWaivedProblem], unexpected: ["new retired-v1 problem"] }
);
assert.match(WINDOWS_CORE_SMOKE_FILTERSET, /test\(=model::tests::windows_live_graph_root_move_is_denied_without_rebinding\)/);
assert.match(WINDOWS_CORE_SMOKE_FILTERSET, /test\(=model::tests::windows_direct_publication_event_waits_for_inflight_writer_receipt\)/);
assert.doesNotMatch(WINDOWS_CORE_SMOKE_FILTERSET, /all\(\)|fast_commit/);
assert.equal(LINUX_TINE_CORE_SHARD_COUNT, 4);

// A ledger that excludes a test BY NAME must name a test that exists. The
// release-selection contract already proves this, but only in hosted CI and only
// after a full compile, so a rename can ride master for a whole campaign first.
// Not hypothetical: Q1 of the query campaign renamed
// managed_graph_search_accounts_for_pending_overlay_metadata_separately and left
// the release ledger naming the dead test. The filterset term test(=<dead name>)
// then excludes nothing, the observed exclusion set stops matching the contract,
// and EVERY master push fails on a message about an exclusion contract rather
// than about the rename that caused it. This scan is the cheap local copy of
// that proof: it runs with no cargo build at all.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const coreSourceRoot = path.join(repoRoot, "crates/tine-core/src");
const coreSourceText = fs
  .readdirSync(coreSourceRoot, { recursive: true })
  .filter((entry) => typeof entry === "string" && entry.endsWith(".rs"))
  .map((entry) => fs.readFileSync(path.join(coreSourceRoot, entry), "utf8"))
  .join("\n");
const declaredCoreTestFns = new Set(
  [...coreSourceText.matchAll(/\bfn\s+([A-Za-z0-9_]+)\s*\(/g)].map((match) => match[1])
);
for (const [ledger, names] of [
  ["KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES in tine-core-nextest-contract.mjs", KNOWN_RED_TINE_CORE_EXCLUDED_TEST_NAMES],
  ["linuxAdditionalKnownRedTestNames in release-ci-exception.json", ONE_RELEASE_CI_EXCEPTION.linuxAdditionalKnownRedTestNames],
]) {
  const rotted = names.filter((name) => !declaredCoreTestFns.has(name.split("::").pop()));
  assert.deepEqual(
    rotted,
    [],
    `${ledger} names ${rotted.length} test(s) that crates/tine-core/src no longer declares: `
      + `${rotted.join(", ")}. An exclusion by name excludes nothing once the name is dead, which `
      + "breaks the Linux release exclusion contract on every master push. Update the ledger in "
      + "the same commit as the rename, and when a test has recovered drop its name outright "
      + "rather than carrying a green name forward as known-red."
  );
}
// Historical waiver names describe retired architectures. They must not remove
// any current witness, even when the helper is asked about the old version.
assert.deepEqual(windowsCoreSmokeTestNames(ONE_RELEASE_CI_EXCEPTION_VERSION), ordinaryCoreSmokeTests);
for (const name of ordinaryCoreSmokeTests) {
  const leaf = name.split("::").at(-1);
  const integrationSource = fs.readFileSync(path.join(repoRoot, "crates/tine-core/tests/oplog_store.rs"), "utf8");
  assert.ok(declaredCoreTestFns.has(leaf) || integrationSource.includes(`fn ${leaf}(`), `Windows required witness no longer exists: ${name}`);
}

console.log("tine-core nextest contract fixture tests passed.");
