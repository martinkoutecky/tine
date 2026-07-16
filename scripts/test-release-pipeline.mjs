#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { assembleCandidate } from "./assemble-release-candidate.mjs";
import {
  mirrorWindowsDevToolsActivePortOnce,
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
const preflight = fs.readFileSync(path.join(process.cwd(), "scripts/check-release-preflight.mjs"), "utf8");
const e2eRunner = fs.readFileSync(path.join(process.cwd(), "scripts/run-e2e.mjs"), "utf8");
const printSecurity = fs.readFileSync(path.join(process.cwd(), "scripts/e2e-print-security.mjs"), "utf8");
const referenceParity = fs.readFileSync(path.join(process.cwd(), "scripts/e2e-og-parity-references.mjs"), "utf8");
const windowsScenarios = [
  "e2e-windows-smoke.mjs",
  "e2e-og-parity-references.mjs",
  "e2e-page-properties.mjs",
  "e2e-page-trailing-block.mjs",
  "e2e-pdf-logseq.mjs",
  "e2e-print-security.mjs",
];

// Architecture guard: the expensive Linux release build must test that exact
// binary before it can be staged for the atomic assembler/publisher. Windows
// consumes the staged portable binary in independent advisory jobs that neither
// serialize assembly nor hide one runner-wide 0/N failure.
const linuxGate = releaseWorkflow.indexOf("Gate Linux x64 on the complete real-app regression catalog");
const stageLane = releaseWorkflow.indexOf("Stage immutable release artifact");
assert(linuxGate >= 0, "release workflow is missing the Linux real-app gate");
assert(stageLane > linuxGate, "release lane is staged before the Linux real-app gate");
assert.match(
  releaseWorkflow,
  /windows-smoke:\n    needs: \[preflight, build\][\s\S]*?if: \$\{\{ always\(\) && needs\.preflight\.result == 'success' && needs\.build\.result != 'cancelled' \}\}[\s\S]*?continue-on-error: true[\s\S]*?name: release-windows-x64[\s\S]*?name: release-e2e-frontend-windows-x64[\s\S]*?npm run e2e:windows:smoke -- --scenario=\$\{\{ matrix\.scenario \}\}/,
  "Windows advisory scenarios do not consume the staged app independently of assembly"
);
assert.match(
  uiE2eWorkflow,
  /windows_scenario == 'all'[\s\S]*?\["windows-core","og-parity-references","page-properties","page-trailing-block","pdf-logseq","print-security"\]/,
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
assert.ok(
  e2eRunner.includes("return /BadWindow \\(invalid Window parameter\\)/.test(combined)")
    && e2eRunner.includes("xdo_get_active_window reported an error"),
  "the release runner does not retry a hosted Quick Capture active-window race"
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
  assert.deepEqual(webdriverServerArgs(4444, 4445, "/driver", "win32"), ["--port=4444"]);
  assert.deepEqual(webdriverServerArgs(4444, 4445, "/driver", "linux"), [
    "--port", "4444", "--native-port", "4445", "--native-driver", "/driver",
  ]);
  const nestedPort = path.join(temporary, "webview2", "fixture-session", "EBWebView", "DevToolsActivePort");
  fs.mkdirSync(path.dirname(nestedPort), { recursive: true });
  fs.writeFileSync(nestedPort, "12345\n/devtools/browser/fixture\n");
  assert.equal(mirrorWindowsDevToolsActivePortOnce(path.join(temporary, "webview2")), 1);
  assert.equal(
    fs.readFileSync(path.join(temporary, "webview2", "fixture-session", "DevToolsActivePort"), "utf8"),
    "12345\n/devtools/browser/fixture\n",
  );
  const profileSnapshot = windowsWebviewProfileSnapshot(path.join(temporary, "webview2"));
  assert.ok(profileSnapshot.files.some((entry) => entry.path === "fixture-session/DevToolsActivePort"));
  assert.ok(profileSnapshot.files.some((entry) => entry.path === "fixture-session/EBWebView/DevToolsActivePort"));
  if (priorWebviewRoot === undefined) delete process.env.E2E_WEBVIEW_USER_DATA_ROOT;
  else process.env.E2E_WEBVIEW_USER_DATA_ROOT = priorWebviewRoot;
  for (const script of windowsScenarios) {
    const source = fs.readFileSync(path.join(process.cwd(), "scripts", script), "utf8");
    assert.match(source, /import \{[^}]*tauriCapabilities[^}]*\} from "\.\/e2e-capabilities\.mjs";/);
    assert.match(source, /capabilities: tauriCapabilities\(APP/);
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

console.log("Release pipeline fixture tests passed (workflow gate + valid + 5 fail-closed cases).");
