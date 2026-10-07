#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { IDENTITIES, IDENTITY } from "./lib/app-identity.mjs";
import { BETA_TAG } from "./release-policy.mjs";
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
// I-12: product spelling comes only from the identity switch, for both ships.
for (const identity of Object.values(IDENTITIES)) {
  const names = releaseLayout(version, identity);
  const prefix = identity.productName.replace(/\s+/g, "-");
  assert.ok(names.lanes["windows-x64"].assets.includes(`${prefix}_${version}_x64-setup.exe`),
    "release layout must derive installer names from the selected identity");
  assert.ok(names.allAssets.every((name) => !/\s/.test(name)), "published asset names contain spaces");
}
const releaseWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/release.yml"), "utf8");
const ciWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/ci.yml"), "utf8");
const uiE2eWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/ui-e2e.yml"), "utf8");
const flatpakWorkflow = fs.readFileSync(path.join(process.cwd(), ".github/workflows/flatpak.yml"), "utf8");
const flatpakMetadataWorkflow = fs.readFileSync(
  path.join(process.cwd(), ".github/workflows/flatpak-metadata.yml"),
  "utf8"
);
assert.match(releaseWorkflow, /name: Install Linux dependencies[\s\S]*?apt-get install[\s\S]*?\bfaketime\b/,
  "release Linux must install faketime for the blocking journal-rollover clock journey");
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

// GH #275: retain the Windows x86 release on both app identities.
for (const identity of Object.values(IDENTITIES)) {
  const names = releaseLayout(version, identity);
  const product = identity.productName.replace(/\s+/g, "-");
  assert.deepEqual(names.lanes["windows-x86"]?.assets, [
    `${product}_${version}_x86-setup.exe`,
    `${product}_${version}_x86-setup.exe.sig`,
    `${product}_${version}_x86-portable.zip`,
  ], "GH #275: Windows x86 must ship installer and portable assets; imitate release-layout.mjs");
  assert.deepEqual(names.lanes["windows-x86"].platforms, {},
    "experimental x86 remains manual-update only");
}
assert.match(releaseWorkflow,
  /lane: windows-x86[\s\S]*?--target i686-pc-windows-msvc[\s\S]*?rust-targets: "i686-pc-windows-msvc"[\s\S]*?win-arch: x86[\s\S]*?win-exe-dir: target\/i686-pc-windows-msvc\/release/,
  "GH #275: release.yml must retain the Windows x86 cross-build");

assert.equal(layout.allAssets.length, 27, "release layout must retain its exact 27-asset inventory");
assert.equal(layout.platformAssets.length, 26, "release layout must retain its exact platform-asset inventory");
assert.equal(
  Object.keys(layout.updaterPlatforms).length,
  12,
  "AppImage update metadata must not add a Tauri updater platform"
);
assert.ok(
  layout.lanes["linux-x64"].assets.includes(`${IDENTITY.productName.replace(/\s+/g, "-")}_${version}_amd64.AppImage.zsync`),
  "linux-x64 is missing its AppImage update metadata"
);
assert.ok(
  layout.lanes["linux-arm64"].assets.includes(`${IDENTITY.productName.replace(/\s+/g, "-")}_${version}_aarch64.AppImage.zsync`),
  "linux-arm64 is missing its AppImage update metadata"
);
assert.match(releaseWorkflow, /release-workflow-inputs.mjs "\$\{\{ matrix\.lane \}\}" >> "\$GITHUB_ENV"/,
  "release workflow must derive bundle names and AppImage update information through the layout door");
assert.doesNotMatch(releaseWorkflow, /Tine_|appimage-update-info:/,
  "I-12: release.yml must not spell stable asset names; use release-workflow-inputs.mjs");
// Beta ships an installable x86_64 Flatpak bundle: a hard release prerequisite
// (a build failure blocks assembly), named by the layout and never an updater
// target. Earlier PV1 scope excluded it (Martin, 2026-10-06 reversed that).
for (const identity of Object.values(IDENTITIES)) {
  const names = releaseLayout(version, identity);
  const product = identity.productName.replace(/\s+/g, "-");
  assert.deepEqual(names.lanes["flatpak-x64"]?.assets, [`${product}_${version}_x86_64.flatpak`],
    "the Flatpak bundle is named like the other assets, from the identity switch");
  assert.deepEqual(names.lanes["flatpak-x64"].platforms, {}, "latest.json must never reference the Flatpak");
  assert.equal(names.lanes["flatpak-x64"].sourceAssets[`${product}_${version}_x86_64.flatpak`],
    `${product}_${version}_x86_64.flatpak`, "the staged bundle is already canonical");
}
assert.ok(RELEASE_LANES.includes("flatpak-x64"), "assembly must require the Flatpak lane");
assert.doesNotMatch(releaseWorkflow, /check-flatpak-/,
  "release.yml still does not run the manifest-source drift checks (they stay in ci)");
assert.ok(!Object.keys(layout.updaterPlatforms).some((platform) => /flatpak/i.test(platform)),
  "the updater must not reference the Flatpak");
assert.match(releaseWorkflow,
  /\n  flatpak:\n    needs: preflight\n    uses: \.\/\.github\/workflows\/flatpak\.yml\n    with:\n      identity: experiment\n/,
  "release.yml must call flatpak.yml as a parallel build job with the Beta identity");
assert.match(releaseWorkflow,
  /\n  stage-flatpak:\n    needs: \[preflight, flatpak\][\s\S]*?release-workflow-inputs\.mjs flatpak-x64[\s\S]*?stage-release-lane\.mjs flatpak-x64[\s\S]*?name: release-flatpak-x64/,
  "the Flatpak bundle must be named by the layout and staged like every other lane");
assert.match(releaseWorkflow, /assemble:\n    needs: \[preflight, build, android, stage-flatpak\]/,
  "assembly must wait for (and so be blocked by) the Flatpak lane");
assert.match(flatpakWorkflow, /workflow_call:\n    inputs:\n      identity:/, "flatpak.yml needs an identity input to be callable");
assert.match(flatpakWorkflow, /derive-flatpak-identity\.mjs --identity "\$FLATPAK_IDENTITY"/,
  "the Flatpak workflow must derive its manifest through the identity-aware script");
assert.match(
  releaseWorkflow,
  /name: Verify Linux AppImage update information[\s\S]*?\.\/src-tauri\/\$zsync_name[\s\S]*?readelf --string-dump=\.upd_info "\$appimage"[\s\S]*?gh-releases-zsync\|/,
  "release workflow verify step must search src-tauri/ for the .zsync (appimagetool writes it into the build CWD) and fail closed when update information is absent"
);
assert.match(
  releaseWorkflow,
  /name: Verify Android 9 native-loader compatibility[\s\S]*?unzip -p "\$apk" lib\/arm64-v8a\/libtine_lib\.so[\s\S]*?readelf --dyn-syms --wide[\s\S]*?renameat2/,
  "Android release packaging must inspect the final APK native library and reject the API-30 renameat2 wrapper"
);

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
assert.match(
  ciWorkflow,
  /pr-validation:[\s\S]*?tool: wasm-pack@0\.15\.0[\s\S]*?name: Committed lsdoc WASM contract is current[\s\S]*?check-wasm-pin\.mjs[\s\S]*?name: F-Droid clean-source WASM rebuild succeeds[\s\S]*?npm run build:wasm[\s\S]*?check-wasm-pin\.mjs/,
  "pull requests do not validate both committed and clean-source rebuilt WASM"
);
assert.match(
  ciWorkflow,
  /test:[\s\S]*?name: Full CI \/ Linux tests and release contracts[\s\S]*?tool: wasm-pack@0\.15\.0[\s\S]*?name: Committed lsdoc WASM contract is current[\s\S]*?check-wasm-pin\.mjs[\s\S]*?name: F-Droid clean-source WASM rebuild succeeds[\s\S]*?npm run build:wasm[\s\S]*?check-wasm-pin\.mjs/,
  "full release CI does not validate both committed and clean-source rebuilt WASM"
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
  /test:\n    name: Full CI \/ Linux tests and release contracts[\s\S]*?uses: dtolnay\/rust-toolchain@\d+\.\d+\.\d+\n        with:\n          targets: wasm32-unknown-unknown[\s\S]*?name: Standalone plugin template builds and conforms\n        run: npm run plugin:template-check/,
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
  /windows_scenario == 'all'[\s\S]*?\["windows-core","page-properties","page-trailing-block","pdf-logseq","print-security","tab-overflow"\]/,
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
  /assemble:\n    needs: \[preflight, build, android, stage-flatpak\]/,
  "candidate assembly accidentally waits for advisory Windows scenarios"
);
assert.match(releaseWorkflow, /name: Upload Windows E2E evidence[\s\S]*?if: always\(\)/);

// GH #650: Beta macOS builds were unsigned and Gatekeeper refused to open them.
// The macOS lane is fail-closed Developer ID signing plus notarization, with the
// same shape as the stable workflow; every other lane never sees an APPLE_* secret.
assert.match(
  releaseWorkflow,
  /name: Prepare macOS signing and notarization credentials[\s\S]*?if: matrix\.lane == 'macos-universal'[\s\S]*?APPLE_CERTIFICATE: \$\{\{ secrets\.APPLE_CERTIFICATE \}\}[\s\S]*?APPLE_API_PRIVATE_KEY: \$\{\{ secrets\.APPLE_API_PRIVATE_KEY \}\}[\s\S]*?security create-keychain[\s\S]*?security set-keychain-settings -lut 21600[\s\S]*?security import "\$p12"[\s\S]*?-f pkcs12[\s\S]*?security find-identity[\s\S]*?chmod 600 "\$key_path"[\s\S]*?APPLE_API_KEY_PATH=\$key_path/,
  "macOS release signing does not explicitly install the Developer ID identity or protect the temporary App Store Connect key"
);
assert.match(
  releaseWorkflow,
  /\[ -z "\$\{!name:-\}" \][\s\S]*?required macOS signing secret \$name is missing/,
  "the macOS lane must refuse to build when a signing secret is empty (an empty APPLE_* value breaks codesign)"
);
assert.match(
  releaseWorkflow,
  /name: Build Tauri bundles\n\s+if: matrix\.lane != 'macos-universal'[\s\S]*?run: node scripts\/build-release-bundles\.mjs -- \$\{\{ matrix\.args \}\}\n[\s\S]*?name: Build signed and notarized macOS bundles\n\s+if: matrix\.lane == 'macos-universal'[\s\S]*?APPLE_SIGNING_IDENTITY: \$\{\{ secrets\.APPLE_SIGNING_IDENTITY \}\}[\s\S]*?APPLE_API_ISSUER: \$\{\{ secrets\.APPLE_API_ISSUER \}\}[\s\S]*?run: node scripts\/build-release-bundles\.mjs -- \$\{\{ matrix\.args \}\}/,
  "Apple signing secrets are not isolated to the macOS release lane, or a lane bypasses the Beta packaging guard"
);
const macosBuildBlock = releaseWorkflow.match(
  /name: Build signed and notarized macOS bundles[\s\S]*?run: node scripts\/build-release-bundles\.mjs -- \$\{\{ matrix\.args \}\}/
)?.[0] ?? "";
assert.doesNotMatch(
  macosBuildBlock.replace(/^\s*#.*$/gm, ""),
  /APPLE_CERTIFICATE(?:_PASSWORD)?:/,
  "the macOS Tauri build must use the explicitly installed identity instead of re-importing the PKCS#12 file"
);
assert.match(
  releaseWorkflow,
  /name: Verify macOS signature and stapled notarization ticket[\s\S]*?hdiutil verify[\s\S]*?hdiutil attach[\s\S]*?find "\$mount"[\s\S]*?codesign --verify --deep --strict[\s\S]*?Authority=Developer ID Application:[\s\S]*?TeamIdentifier=\$APPLE_TEAM_ID[\s\S]*?xcrun stapler validate[\s\S]*?spctl --assess/,
  "the macOS lane must mount the shipped DMG and prove its app signing, notarization, and Gatekeeper acceptance"
);
assert.ok(
  releaseWorkflow.indexOf("name: Build signed and notarized macOS bundles")
    < releaseWorkflow.indexOf("name: Verify macOS signature and stapled notarization ticket")
    && releaseWorkflow.indexOf("name: Verify macOS signature and stapled notarization ticket")
      < releaseWorkflow.indexOf("name: Stage immutable release artifact"),
  "the macOS signature and ticket must be verified before the lane is staged for assembly"
);
assert.match(
  releaseWorkflow,
  /name: Remove macOS signing material\n\s+if: always\(\) && matrix\.lane == 'macos-universal'[\s\S]*?security delete-keychain[\s\S]*?app-store-connect-private-keys/,
  "temporary macOS signing material is not cleaned after failures"
);
assert.doesNotMatch(
  releaseWorkflow.replace(/^\s*#.*$/gm, ""),
  /\n {0,8}APPLE_[A-Z_]+:/,
  "an APPLE_* variable must only be set at step level (never job- or workflow-level) so non-macOS lanes never receive it"
);
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
    // A missing Flatpak bundle (its lane artifact absent) fails assembly, so a
    // Flatpak build failure blocks the release like any other platform.
    const base = path.join(temporary, "missing-flatpak");
    const input = makeInput(base);
    fs.rmSync(path.join(input, "release-flatpak-x64"), { recursive: true });
    assert.throws(() => assemble(input, path.join(base, "output")), /missing release lanes: flatpak-x64/);
  }
  {
    // A lane whose bundle is renamed or absent from its fragment is refused too.
    const base = path.join(temporary, "wrong-flatpak-name");
    const input = makeInput(base);
    const [flatpak] = layout.lanes["flatpak-x64"].assets;
    fs.renameSync(path.join(input, "release-flatpak-x64", flatpak), path.join(input, "release-flatpak-x64", "other.flatpak"));
    assert.throws(() => assemble(input, path.join(base, "output")), /ENOENT/);
  }
  {
    const base = path.join(temporary, "missing-signature");
    const input = makeInput(base);
    fs.rmSync(path.join(input, "release-windows-x64", layout.lanes["windows-x64"].assets.find((name) => name.endsWith("-setup.exe.sig"))));
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

await import("./test-release-identity.mjs");
