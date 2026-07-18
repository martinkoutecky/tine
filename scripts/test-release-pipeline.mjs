#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { assembleCandidate } from "./assemble-release-candidate.mjs";
import {
  collectGithubPages,
  REQUIRED_FULL_CI_JOBS,
  selectExactCiEvidence,
} from "./ci-evidence-lib.mjs";
import {
  tauriCapabilities,
  webdriverServerArgs,
  windowsWebviewProfileSnapshot,
} from "./e2e-capabilities.mjs";
import { candidateProblems, releaseLayout, RELEASE_LANES } from "./release-layout.mjs";

const version = "0.5.6";
const commit = "a".repeat(40);
const repository = "martinkoutecky/tine";
const layout = releaseLayout(version);
const releaseWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/release.yml"), "utf8");
const ciWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/ci.yml"), "utf8");
const uiE2eWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/ui-e2e.yml"), "utf8");
const flatpakWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/flatpak.yml"), "utf8");
const flatpakMetadataWorkflow = fs.readFileSync(
  path.join(process.cwd(), ".github/workflows/flatpak-metadata.yml"),
  "utf8"
);
const preflight = fs.readFileSync(path.join(process.cwd(), "scripts/check-release-preflight.mjs"), "utf8");
const e2eRunner = fs.readFileSync(path.join(process.cwd(), "scripts/run-e2e.mjs"), "utf8");
const receiptHelper = fs.readFileSync(path.join(process.cwd(), "scripts/build-e2e-receipt.mjs"), "utf8");
const buildInputs = fs.readFileSync(path.join(process.cwd(), "scripts/build-e2e-inputs.mjs"), "utf8");
const printSecurity = fs.readFileSync(path.join(process.cwd(), "scripts/e2e-print-security.mjs"), "utf8");
const referenceParity = fs.readFileSync(path.join(process.cwd(), "scripts/e2e-og-parity-references.mjs"), "utf8");
const windowsScenarios = [
  "e2e-windows-smoke.mjs",
  "e2e-og-parity-references.mjs",
  "e2e-page-properties.mjs",
  "e2e-page-trailing-block.mjs",
  "e2e-pdf-logseq.mjs",
  "e2e-print-security.mjs",
  "e2e-tab-overflow.mjs",
];

const successfulFullCiRun = {
  id: 1234,
  event: "workflow_dispatch",
  head_sha: commit,
  status: "completed",
  conclusion: "success",
  html_url: "https://example.invalid/actions/runs/1234",
};
const successfulFullCiJobs = REQUIRED_FULL_CI_JOBS.map((name) => ({ name, conclusion: "success" }));

// Architecture guard: the expensive Linux release build must test that exact
// binary before it can be staged for the atomic assembler/publisher. Windows
// consumes the staged portable binary in independent advisory jobs that neither
// serialize assembly nor hide one runner-wide 0/N failure.
assert.doesNotMatch(ciWorkflow, /\n  push:/, "ordinary CI still runs automatically on pushes");
assert.match(
  ciWorkflow,
  /workflow_dispatch:[\s\S]*?scope:[\s\S]*?options:[\s\S]*?- full[\s\S]*?- windows[\s\S]*?- android[\s\S]*?- performance/,
  "manual CI does not expose full and focused proof scopes"
);
assert.match(
  ciWorkflow,
  /pull_request:[\s\S]*?paths-ignore:[\s\S]*?"\*\*\/\*\.md"/,
  "docs-only pull requests still start app validation"
);
for (const name of REQUIRED_FULL_CI_JOBS) {
  assert.ok(ciWorkflow.includes(`name: ${name}`), `CI workflow is missing stable evidence job ${name}`);
}
assert.match(
  ciWorkflow,
  /test:[\s\S]*?name: Full CI \/ Linux tests and release contracts[\s\S]*?inputs\.scope == 'full'/,
  "the Linux full-CI evidence job can run in a focused dispatch"
);
assert.match(
  ciWorkflow,
  /test:\n    name: Full CI \/ Linux tests and release contracts[\s\S]*?uses: dtolnay\/rust-toolchain@stable\n        with:\n          targets: wasm32-unknown-unknown[\s\S]*?name: Standalone plugin template builds and conforms\n        run: npm run plugin:template-check/,
  "the Linux full-CI plugin-template check does not install the WASM target"
);
assert.match(
  ciWorkflow,
  /windows-compile:[\s\S]*?inputs\.scope == 'full'[\s\S]*?inputs\.scope == 'windows'/,
  "the Windows lane cannot distinguish full and focused dispatches"
);
assert.match(
  ciWorkflow,
  /android-core-compile:[\s\S]*?inputs\.scope == 'full'[\s\S]*?inputs\.scope == 'android'/,
  "the Android lane cannot distinguish full and focused dispatches"
);
assert.match(
  ciWorkflow,
  /bench:[\s\S]*?inputs\.scope == 'full'[\s\S]*?inputs\.scope == 'performance'/,
  "the performance lane cannot distinguish full and focused dispatches"
);
assert.doesNotMatch(flatpakWorkflow, /\n  push:/, "the expensive Flatpak build still runs automatically on pushes");
assert.match(flatpakMetadataWorkflow, /\n  pull_request:/, "lightweight Flatpak metadata validation is not on PRs");
assert.doesNotMatch(flatpakMetadataWorkflow, /\n  push:/, "Flatpak metadata validation still runs after merge");
assert.match(
  releaseWorkflow,
  /permissions:[\s\S]*?contents: read[\s\S]*?actions: read[\s\S]*?preflight:[\s\S]*?name: Require exact-SHA full CI evidence[\s\S]*?node scripts\/check-ci-evidence\.mjs[\s\S]*?uses: dtolnay\/rust-toolchain/,
  "release packaging does not fail closed on exact-SHA full CI evidence before expensive setup"
);

assert.equal(
  selectExactCiEvidence(commit, [{ run: successfulFullCiRun, jobs: successfulFullCiJobs }]).run.id,
  successfulFullCiRun.id
);
assert.throws(
  () => selectExactCiEvidence("b".repeat(40), [{ run: successfulFullCiRun, jobs: successfulFullCiJobs }]),
  /No successful full CI evidence for exact SHA/
);
assert.throws(
  () => selectExactCiEvidence(commit, [{
    run: { ...successfulFullCiRun, event: "pull_request" },
    jobs: successfulFullCiJobs,
  }]),
  /run event is pull_request, not workflow_dispatch/
);
assert.throws(
  () => selectExactCiEvidence(commit, [{ run: successfulFullCiRun, jobs: successfulFullCiJobs.slice(0, 1) }]),
  /Full CI \/ Windows compile and core tests concluded missing/
);
assert.throws(
  () => selectExactCiEvidence(commit, [{
    run: successfulFullCiRun,
    jobs: successfulFullCiJobs.map((job) => ({
      ...job,
      conclusion: job.name === REQUIRED_FULL_CI_JOBS[3] ? "failure" : job.conclusion,
    })),
  }]),
  /Full CI \/ performance A\/B concluded failure/
);
const paginationCalls = [];
assert.deepEqual(
  await collectGithubPages(async (page) => {
    paginationCalls.push(page);
    return { jobs: page === 1 ? [{ id: 1 }, { id: 2 }] : [{ id: 3 }] };
  }, "jobs", { perPage: 2 }),
  [{ id: 1 }, { id: 2 }, { id: 3 }]
);
assert.deepEqual(paginationCalls, [1, 2], "GitHub pagination did not stop after the short final page");

const linuxGate = releaseWorkflow.indexOf("Gate Linux x64 on the complete real-app regression catalog");
const stageLane = releaseWorkflow.indexOf("Stage immutable release artifact");
assert(linuxGate >= 0, "release workflow is missing the Linux real-app gate");
assert(stageLane > linuxGate, "release lane is staged before the Linux real-app gate");
assert.match(
  receiptHelper,
  /buildInputState[\s\S]*?refusing receipt: HEAD changed while building[\s\S]*?build-input state changed while building[\s\S]*?binary does not embed current production frontend[\s\S]*?buildInputDigest/,
  "the receipt helper does not bind a build to its pre-build source state and embedded frontend"
);
assert.match(buildInputs, /export function buildInputState\(/, "buildInputState is not exported");
assert.match(buildInputs, /ls-files[\s\S]*?digest/, "build-input state is not bound to git ls-files and a digest");
assert.match(
  e2eRunner,
  /buildInputState[\s\S]*?const e2eMode = process\.env\.TINE_E2E_MODE \?\? "ordinary";[\s\S]*?buildInputDigest[\s\S]*?build receipt is required at/,
  "run-e2e does not default to ordinary mode and require a receipt"
);
assert.doesNotMatch(e2eRunner, /GITHUB_SHA|TINE_E2E_ALLOW_UNRECEIPTED_APP/);
assert.match(
  e2eRunner,
  /if \(e2eMode === "release"\) \{[\s\S]*?contract\.class !== "flexible-presentation-heuristic"/,
  "release mode does not block every safety, core-operation, and stateful-UX failure"
);
assert.match(
  uiE2eWorkflow,
  /Snapshot Linux E2E candidate inputs[\s\S]*?Write Linux E2E candidate receipt[\s\S]*?Snapshot Windows E2E candidate inputs[\s\S]*?Write Windows E2E candidate receipt/,
  "manually dispatched raw Linux and Windows builds do not create receipts"
);
assert.match(
  releaseWorkflow,
  /Snapshot Linux E2E candidate inputs[\s\S]*?Write Linux E2E candidate receipt[\s\S]*?TINE_E2E_MODE: release[\s\S]*?npm run e2e:linux:release/,
  "the release Linux E2E candidate does not use a pre-build receipt or release mode"
);
assert.match(
  releaseWorkflow,
  /Snapshot Windows E2E candidate inputs[\s\S]*?--tauri-manifest-normalization[\s\S]*?Write Windows E2E candidate receipt[\s\S]*?release-e2e-receipt-windows-x64[\s\S]*?TINE_E2E_BUILD_RECEIPT=[\s\S]*?TINE_E2E_MODE: release/,
  "the advisory release Windows E2E run does not normalize the exact Tauri manifest before receiving its receipt in release mode"
);
assert.match(
  releaseWorkflow,
  /windows-smoke:\n    needs: \[preflight, build\][\s\S]*?if: \$\{\{ always\(\) && needs\.preflight\.result == 'success' && needs\.build\.result != 'cancelled' \}\}[\s\S]*?continue-on-error: true[\s\S]*?name: release-windows-x64[\s\S]*?name: release-e2e-frontend-windows-x64[\s\S]*?npm run e2e:windows:smoke -- --scenario=\$\{\{ matrix\.scenario \}\}/,
  "Windows advisory scenarios do not consume the staged app independently of assembly"
);
assert.match(
  uiE2eWorkflow,
  /windows_scenario == 'all'[\s\S]*?\["windows-core","og-parity-references","page-properties","page-trailing-block","pdf-logseq","print-security","tab-overflow"\]/,
  "the focused UI workflow cannot fan out all Windows scenarios explicitly"
);
assert.doesNotMatch(
  uiE2eWorkflow,
  /name: Run Windows WebView2 smoke\n\s+continue-on-error:/,
  "the focused Windows workflow hides a 0\/N scenario result behind a green job"
);
assert.match(
  releaseWorkflow,
  /name: Upload exact Windows x64 frontend proof[\s\S]*?if: matrix\.lane == 'windows-x64'[\s\S]*?name: release-e2e-frontend-windows-x64[\s\S]*?path: dist/,
  "the release build does not preserve the exact frontend needed to validate the staged Windows executable"
);
assert.match(
  releaseWorkflow,
  /assemble:\n    needs: \[preflight, flatpak, build, android\]/,
  "candidate assembly accidentally waits for advisory Windows scenarios"
);
assert.match(releaseWorkflow, /name: Upload Windows E2E evidence[\s\S]*?if: always\(\)/);
assert.match(
  e2eRunner,
  /if \(process\.platform === "linux"\) \{\n      env\.WEBKIT_DRIVER = process\.env\.WEBKIT_DRIVER \|\| "\/usr\/bin\/WebKitWebDriver";/,
  "the suite runner leaks Linux WebKitWebDriver into Windows"
);
assert.match(
  e2eRunner,
  /TAURI_DRIVER: process\.env\.TAURI_DRIVER \|\| \(process\.platform === "win32" \? "msedgedriver\.exe" : "tauri-driver"\)/,
  "Windows scenarios still route native WebView2 through the unnecessary Tauri proxy"
);
const driverTransportFailureSource = e2eRunner.match(
  /function isRetryableDriverTransportFailure\(output, errors, timedOut\) \{[\s\S]*?\n\}/
);
assert.ok(driverTransportFailureSource, "the release runner is missing its WebDriver transport retry predicate");
const isRetryableDriverTransportFailure = new Function(
  `${driverTransportFailureSource[0]}\nreturn isRetryableDriverTransportFailure;`
)();
assert.equal(
  isRetryableDriverTransportFailure(
    'WebDriverError: invalid session id when running\n"element/.../property/value" with method "GET"\nError: Arrow Down did not cross from the page header into the first body block',
    "",
    false
  ),
  true,
  "the hosted terminal WebDriver invalid-session failure is not retried"
);
assert.equal(
  isRetryableDriverTransportFailure("WebDriverError: GET /session failed: UND_ERR_SOCKET", "", false),
  true,
  "existing WebDriver socket transport failures are not retried"
);
assert.equal(
  isRetryableDriverTransportFailure("Arrow Down assertion failed: invalid session id", "", false), false,
  "generic invalid-session text without a WebDriver error must not be retried"
);
assert.equal(
  isRetryableDriverTransportFailure("WebDriverError: element assertion failed", "", false), false,
  "arbitrary WebDriver assertion failures must not be retried"
);
assert.equal(
  isRetryableDriverTransportFailure("Arrow Down did not cross from the page header into the first body block", "", false),
  false,
  "product assertion failures without a WebDriver error must not be retried"
);
assert.equal(
  isRetryableDriverTransportFailure("WebDriverError: invalid session id", "", true), false,
  "scenario timeouts must not be retried as driver infrastructure failures"
);
const nativeHarnessFailureSource = e2eRunner.match(
  /function isRetryableNativeHarnessFailure\(id, output, errors, timedOut\) \{[\s\S]*?\n\}/
);
assert.ok(nativeHarnessFailureSource, "the release runner is missing its Quick Capture native-harness retry predicate");
const isRetryableNativeHarnessFailure = new Function(
  `${nativeHarnessFailureSource[0]}\nreturn isRetryableNativeHarnessFailure;`
)();
assert.equal(
  isRetryableNativeHarnessFailure(
    "capture",
    "BadWindow (invalid Window parameter)\nxdo_get_active_window reported an error",
    "",
    false
  ),
  true,
  "the legacy GTK BadWindow active-window race is not retried"
);
assert.equal(
  isRetryableNativeHarnessFailure(
    "capture",
    "XGetWindowProperty[_NET_ACTIVE_WINDOW] failed (code=1)\nxdo_get_active_window reported an error",
    "",
    false
  ),
  true,
  "the demonstrated xdotool active-window race is not retried"
);
assert.equal(
  isRetryableNativeHarnessFailure("capture", "cold-restart autocomplete assertion failed", "", false),
  false,
  "arbitrary Quick Capture assertion failures must not be retried"
);
assert.match(
  printSecurity,
  /const driverArgs = webdriverServerArgs\([\s\S]*?DRIVER_PORT,[\s\S]*?NATIVE_PORT,[\s\S]*?WEBKIT_DRIVER/,
  "print-security does not select the native WebDriver by platform"
);
assert.match(
  referenceParity,
  /APP_DATA_ROOT = process\.platform === "win32"[\s\S]*?APPDATA: APP_DATA_ROOT,[\s\S]*?LOCALAPPDATA:/,
  "reference parity does not isolate and seed Windows app settings"
);
assert.match(
  e2eRunner,
  /\["og-parity-references", "scripts\/e2e-og-parity-references\.mjs"[\s\S]*?\["capture", "scripts\/e2e-capture\.mjs"/,
  "the release suite does not retain independent reference and Quick Capture proofs"
);
assert.doesNotMatch(
  referenceParity,
  /scripts\/e2e-capture\.mjs/,
  "reference parity nests the independent native Quick Capture process tree"
);
assert.match(
  ciWorkflow,
  /name: Performance baseline policy is current[\s\S]*?releases\/latest[\s\S]*?node scripts\/check-bench-policy\.mjs --expected-previous "\$latest"/,
  "ordinary CI does not compare the performance baseline with the actually published release"
);
assert.match(
  ciWorkflow,
  /bench:[\s\S]*?fetch-depth: 0[\s\S]*?name: Require the rolling baseline to be the latest published release[\s\S]*?releases\/latest[\s\S]*?node scripts\/check-bench-policy\.mjs --expected-previous "\$latest"/,
  "the A/B benchmark job does not validate baseline currency against the published release before measuring"
);
assert.match(
  ciWorkflow,
  /bench:[\s\S]*?node scripts\/bench-ab\.mjs[\s\S]*?--candidate-dir \.[\s\S]*?--immutable-dir \.bench\/immutable[\s\S]*?--previous-dir \.bench\/previous/,
  "the A/B benchmark job does not measure all three versions through the interleaved multi-round harness"
);
assert.match(
  ciWorkflow,
  /name: Performance A\/B multi-round reliability fixtures[\s\S]*?node scripts\/test-bench-ab\.mjs/,
  "ordinary CI does not prove the performance gate rejects metric-level variance"
);
assert.match(
  releaseWorkflow,
  /preflight:[\s\S]*?fetch-depth: 0/,
  "release preflight cannot determine the previous release from a shallow checkout"
);
assert.match(preflight, /check-bench-policy\.mjs/, "release preflight omits the performance-baseline currency guard");

function makeInput(base) {
  const input = path.join(base, "input");
  fs.mkdirSync(input, { recursive: true });
  for (const lane of RELEASE_LANES) {
    const directory = path.join(input, `release-${lane}`);
    fs.mkdirSync(directory, { recursive: true });
    const assets = [];
    for (const name of layout.lanes[lane].assets) {
      const contents = name.endsWith(".sig") ? `signature-${name}\n` : `fixture-${name}\n`;
      fs.writeFileSync(path.join(directory, name), contents);
      const bytes = Buffer.from(contents);
      assets.push({ name, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") });
    }
    const platforms = {};
    for (const [platform, [asset, signatureAsset]] of Object.entries(layout.lanes[lane].platforms)) {
      platforms[platform] = {
        asset,
        signature: fs.readFileSync(path.join(directory, signatureAsset), "utf8").trim(),
      };
    }
    fs.writeFileSync(
      path.join(directory, "release-fragment.json"),
      `${JSON.stringify({ version, commit, lane, assets, platforms }, null, 2)}\n`
    );
  }
  return input;
}

function assemble(input, output) {
  assembleCandidate({
    input,
    output,
    version,
    commit,
    repository,
    pubDate: "2026-07-11T00:00:00.000Z",
  });
}

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-release-pipeline-test-"));
try {
  const priorWebviewRoot = process.env.E2E_WEBVIEW_USER_DATA_ROOT;
  process.env.E2E_WEBVIEW_USER_DATA_ROOT = path.join(temporary, "webview2");
  const windowsCapabilities = tauriCapabilities("C:/Tine.exe", "fixture session", "win32");
  assert.equal(
    windowsCapabilities["ms:edgeOptions"].webviewOptions.userDataFolder,
    path.join(temporary, "webview2", "fixture-session"),
  );
  assert.equal(windowsCapabilities.browserName, "webview2");
  assert.equal(windowsCapabilities["ms:edgeOptions"].binary, "C:/Tine.exe");
  const attachedCapabilities = tauriCapabilities(
    "C:/Tine.exe",
    "fixture session",
    "win32",
    "127.0.0.1:9222",
  );
  assert.equal(attachedCapabilities["ms:edgeOptions"].debuggerAddress, "127.0.0.1:9222");
  assert.equal(attachedCapabilities["ms:edgeOptions"].binary, undefined);
  assert.deepEqual(webdriverServerArgs(4444, 4445, "/driver", "win32"), ["--port=4444"]);
  assert.deepEqual(webdriverServerArgs(4444, 4445, "/driver", "linux"), [
    "--port", "4444", "--native-port", "4445", "--native-driver", "/driver",
  ]);
  const nestedPort = path.join(temporary, "webview2", "fixture-session", "EBWebView", "DevToolsActivePort");
  fs.mkdirSync(path.dirname(nestedPort), { recursive: true });
  fs.writeFileSync(nestedPort, "12345\n/devtools/browser/fixture\n");
  const profileSnapshot = windowsWebviewProfileSnapshot(path.join(temporary, "webview2"));
  assert.ok(profileSnapshot.files.some((entry) => entry.path === "fixture-session/EBWebView/DevToolsActivePort"));
  if (priorWebviewRoot === undefined) delete process.env.E2E_WEBVIEW_USER_DATA_ROOT;
  else process.env.E2E_WEBVIEW_USER_DATA_ROOT = priorWebviewRoot;
  for (const script of windowsScenarios) {
    const source = fs.readFileSync(path.join(process.cwd(), "scripts", script), "utf8");
    assert.match(source, /import \{[^}]*tauriCapabilities[^}]*\} from "\.\/e2e-capabilities\.mjs";/);
    assert.match(source, /startWebdriverApplication\(APP,/);
    assert.match(source, /capabilities: tauriCapabilities\(APP,[^\n]*webviewTarget\.debuggerAddress/);
    assert.match(source, /stopWebdriverApplication\(webviewTarget\)/);
  }

  {
    const base = path.join(temporary, "valid");
    const input = makeInput(base);
    const output = path.join(base, "output");
    assemble(input, output);
    assert.deepEqual(candidateProblems(output, version), []);
  }
  {
    const base = path.join(temporary, "missing-android");
    const input = makeInput(base);
    fs.rmSync(path.join(input, "release-android"), { recursive: true });
    assert.throws(() => assemble(input, path.join(base, "output")), /missing release lanes: android/);
  }
  {
    const base = path.join(temporary, "missing-signature");
    const input = makeInput(base);
    fs.rmSync(path.join(input, "release-windows-x64", `Tine_${version}_x64-setup.exe.sig`));
    assert.throws(() => assemble(input, path.join(base, "output")), /ENOENT/);
  }
  {
    const base = path.join(temporary, "wrong-version");
    const input = makeInput(base);
    const fragmentPath = path.join(input, "release-macos-universal", "release-fragment.json");
    const fragment = JSON.parse(fs.readFileSync(fragmentPath, "utf8"));
    fragment.version = "0.5.7";
    fs.writeFileSync(fragmentPath, JSON.stringify(fragment));
    assert.throws(() => assemble(input, path.join(base, "output")), /version 0\.5\.7, expected 0\.5\.6/);
  }
  {
    const base = path.join(temporary, "duplicate-platform");
    const input = makeInput(base);
    const fragmentPath = path.join(input, "release-windows-x64", "release-fragment.json");
    const fragment = JSON.parse(fs.readFileSync(fragmentPath, "utf8"));
    fragment.platforms["linux-x86_64"] = fragment.platforms["windows-x86_64"];
    fs.writeFileSync(fragmentPath, JSON.stringify(fragment));
    assert.throws(() => assemble(input, path.join(base, "output")), /updater platform contract mismatch/);
  }
  {
    const base = path.join(temporary, "incomplete-updater");
    const input = makeInput(base);
    const output = path.join(base, "output");
    assemble(input, output);
    const updaterPath = path.join(output, "latest.json");
    const updater = JSON.parse(fs.readFileSync(updaterPath, "utf8"));
    delete updater.platforms["windows-aarch64"];
    fs.writeFileSync(updaterPath, JSON.stringify(updater));
    assert(candidateProblems(output, version).some((problem) => problem.includes("windows-aarch64")));
  }
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}

console.log("Release pipeline fixture tests passed (exact-SHA CI gate + release workflow + fail-closed cases).");
