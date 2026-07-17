#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import crypto from "node:crypto";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildInputState } from "./build-e2e-inputs.mjs";
import { windowsWebviewProfileSnapshot } from "./e2e-capabilities.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const contractsPath = path.join(root, "tests/ui-regressions/e2e-contracts.json");
const suiteName = process.argv[2] ?? "linux-smoke";
const only = process.argv.find((arg) => arg.startsWith("--scenario="))?.slice("--scenario=".length);
const app = path.resolve(process.env.TINE_APP || path.join(root, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine"));
const artifactRoot = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "test-results/e2e", suiteName));
const timeoutMs = Number(process.env.E2E_SCENARIO_TIMEOUT_MS || 180_000);
const suiteStartedAt = new Date().toISOString();
function gitOutput(args) {
  const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  if (result.status !== 0) throw result.error || new Error(`git ${args.join(" ")} failed: ${String(result.stderr || "").trim()}`);
  return result.stdout.trim();
}

const checkoutRevision = gitOutput(["rev-parse", "HEAD"]);
const receiptPath = process.env.TINE_E2E_BUILD_RECEIPT
  ? path.resolve(process.env.TINE_E2E_BUILD_RECEIPT)
  : `${app}.build.json`;
const e2eMode = process.env.TINE_E2E_MODE ?? "ordinary";
const allowedContractClasses = new Set(["exact-safety-interoperability", "core-operation", "stateful-ux", "flexible-presentation-heuristic"]);
const allowedStabilities = new Set(["stable", "burn-in", "quarantined"]);

if (!new Set(["ordinary", "release"]).has(e2eMode)) {
  throw new Error(`unknown TINE_E2E_MODE ${JSON.stringify(e2eMode)}; choose ordinary or release`);
}

// Rootless/container fallback for native focus tests. CI images normally
// install openbox + xdotool system-wide; a developer sandbox may instead keep
// their extracted Debian packages outside the repository. Discover that
// workspace-local bundle automatically so the documented npm command remains
// the complete gate rather than requiring a remembered shell incantation.
const portableDeps = path.resolve(process.env.TINE_E2E_DEPS_ROOT || path.join(root, "../.codex-deps/openbox/root"));
const baseProcessEnv = { ...process.env };
if (fs.existsSync(path.join(portableDeps, "usr/bin/openbox")) && fs.existsSync(path.join(portableDeps, "usr/bin/xdotool"))) {
  const lib = path.join(portableDeps, "usr/lib/x86_64-linux-gnu");
  baseProcessEnv.PATH = `${path.join(portableDeps, "usr/bin")}${path.delimiter}${baseProcessEnv.PATH || ""}`;
  baseProcessEnv.LD_LIBRARY_PATH = [lib, baseProcessEnv.LD_LIBRARY_PATH].filter(Boolean).join(path.delimiter);
  baseProcessEnv.XDG_CONFIG_DIRS = [path.join(portableDeps, "etc/xdg"), baseProcessEnv.XDG_CONFIG_DIRS || "/etc/xdg"].join(path.delimiter);
  baseProcessEnv.XDG_DATA_DIRS = [path.join(portableDeps, "usr/share"), baseProcessEnv.XDG_DATA_DIRS || "/usr/local/share:/usr/share"].join(path.delimiter);
}

// xprop lives in x11-utils rather than the Openbox/xdotool dependency bundle.
// Worktrees are one directory deeper than tine-master, so search both normal
// workspace layouts (or honor an explicit override) and add the first complete
// portable bundle to PATH. This keeps `npm run e2e:linux:release` sufficient in
// the rootless Codex sandbox instead of relying on a remembered PATH prefix.
const portableX11Deps = [
  process.env.TINE_E2E_X11_DEPS_ROOT,
  path.join(root, "../.codex-deps/x11-utils/root"),
  path.join(root, "../../.codex-deps/x11-utils/root"),
]
  .filter(Boolean)
  .map((candidate) => path.resolve(candidate))
  .find((candidate) => fs.existsSync(path.join(candidate, "usr/bin/xprop")));
if (portableX11Deps) {
  baseProcessEnv.PATH = `${path.join(portableX11Deps, "usr/bin")}${path.delimiter}${baseProcessEnv.PATH || ""}`;
}

const suites = {
  "plugin-revocation": [
    ["plugin-revocation", "scripts/e2e-plugin-revocation.mjs", {}],
  ],
  "plugin-graph-ownership": [
    ["plugin-graph-ownership", "scripts/e2e-plugin-graph-ownership.mjs", {}],
  ],
  "og-parity-pilot": [
    ["og-parity-references", "scripts/e2e-og-parity-references.mjs", {}],
  ],
  "linux-smoke": [
    ["caret-agenda", "scripts/e2e-caret.mjs", { CARET_MODE: "agenda", CARET_LABEL: "runner" }],
    ["multigraph", "scripts/e2e-multigraph.mjs", {}],
    ["sheets", "scripts/e2e-sheets.mjs", {}],
  ],
  "linux-release": [
    ["wayland-app-id", "scripts/e2e-wayland-app-id.mjs", {}],
    ["caret-agenda", "scripts/e2e-caret.mjs", { CARET_MODE: "agenda", CARET_LABEL: "runner" }],
    ["caret-page", "scripts/e2e-caret.mjs", { CARET_MODE: "page", CARET_LABEL: "runner" }],
    ["click-caret", "scripts/e2e-clickcaret-repro.mjs", {}],
    ["block-select", "scripts/e2e-blockselect.mjs", {}],
    ["block-ref-count", "scripts/e2e-block-ref-count.mjs", {}],
    ["og-parity-references", "scripts/e2e-og-parity-references.mjs", {}],
    ["search-parity", "scripts/e2e-search-parity.mjs", {}],
    ["rename", "scripts/e2e-rename.mjs", {}],
    ["split-history", "scripts/e2e-split-history.mjs", {}],
    ["alias", "scripts/e2e-alias.mjs", {}],
    ["page-properties", "scripts/e2e-page-properties.mjs", {}],
    ["journal-format", "scripts/e2e-journal-format.mjs", {}],
    ["journal-future-feed", "scripts/e2e-journal-future-feed.mjs", {}],
    ["multigraph", "scripts/e2e-multigraph.mjs", {}],
    ["sheets", "scripts/e2e-sheets.mjs", {}],
    ["formula-builder", "scripts/probe-formula-builder.mjs", {}],
    ["selection-wrap", "scripts/e2e-selectwrap.mjs", {}],
    ["tag-autocomplete", "scripts/e2e-tag-autocomplete.mjs", {}],
    ["structured-paste", "scripts/e2e-structured-paste.mjs", {}],
    ["media", "scripts/e2e-media.mjs", {}],
    ["pdf-logseq", "scripts/e2e-pdf-logseq.mjs", { E2E_WINDOW_MANAGER: "openbox" }],
    ["pdf-ownership", "scripts/e2e-pdf-ownership.mjs", {}],
    ["plugin-graph-ownership", "scripts/e2e-plugin-graph-ownership.mjs", {}],
    ["external-assets", "scripts/e2e-external-assets.mjs", {}],
    ["capture", "scripts/e2e-capture.mjs", { E2E_WINDOW_MANAGER: process.env.E2E_WINDOW_MANAGER || "openbox" }],
    ["native-titlebar", "scripts/e2e-native-titlebar.mjs", { E2E_WINDOW_MANAGER: "openbox" }],
    ["page-file-actions", "scripts/e2e-page-file-actions.mjs", {}],
    ["print-security", "scripts/e2e-print-security.mjs", {}],
    ["block-embed", "scripts/e2e-block-embed.mjs", {}],
    ["sidebar-sections", "scripts/e2e-sidebar-sections.mjs", {}],
    ["right-sidebar-collapse", "scripts/e2e-right-sidebar-collapse.mjs", {}],
    ["mobile-drawers", "scripts/e2e-mobile-drawers.mjs", { TINE_E2E_FORCE_MOBILE_DRAWERS: "1" }],
    ["tab-overflow", "scripts/e2e-tab-overflow.mjs", {}],
    ["outline-guide", "scripts/e2e-outline-guide.mjs", {}],
    ["query-workspace", "scripts/e2e-query-workspace.mjs", {}],
    ["empty-query-workspace", "scripts/e2e-empty-query-workspace.mjs", {}],
    ["scrollbars", "scripts/e2e-scrollbars.mjs", {}],
    ["page-trailing-block", "scripts/e2e-page-trailing-block.mjs", {}],
  ],
  "windows-smoke": [
    ["og-parity-references", "scripts/e2e-og-parity-references.mjs", {}],
    ["page-properties", "scripts/e2e-page-properties.mjs", {}],
    ["pdf-logseq", "scripts/e2e-pdf-logseq.mjs", { E2E_WINDOW_MANAGER: "openbox" }],
    ["print-security", "scripts/e2e-print-security.mjs", {}],
    ["windows-core", "scripts/e2e-windows-smoke.mjs", {}],
    ["page-trailing-block", "scripts/e2e-page-trailing-block.mjs", {}],
    ["tab-overflow", "scripts/e2e-tab-overflow.mjs", {}],
  ],
};

if (!suites[suiteName]) {
  console.error(`unknown suite ${suiteName}; choose ${Object.keys(suites).join(", ")}`);
  process.exit(2);
}

function loadSelectedContracts(scenarios) {
  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(contractsPath, "utf8"));
  } catch (error) {
    throw new Error(`could not load E2E contracts: ${error.message}`);
  }
  if (manifest.schemaVersion !== 1 || !manifest.scenarios || Array.isArray(manifest.scenarios) || typeof manifest.scenarios !== "object") {
    throw new Error("E2E contracts must have schemaVersion 1 and a scenarios object");
  }
  const selected = new Map();
  for (const [id, script] of scenarios) {
    const entry = manifest.scenarios[script];
    if (!entry || !Array.isArray(entry.contracts) || entry.contracts.length === 0) {
      throw new Error(`E2E contract missing for selected scenario ${id} (${script})`);
    }
    if (!Array.isArray(entry.acceptableVariations) || !Array.isArray(entry.nonRequirements) || !allowedStabilities.has(entry.stability)) {
      throw new Error(`E2E contract has invalid scenario fields for ${script}`);
    }
    for (const contract of entry.contracts) {
      if (!contract || !allowedContractClasses.has(contract.class) || typeof contract.userOutcome !== "string" || typeof contract.blocking !== "boolean") {
        throw new Error(`E2E contract has invalid contract fields for ${script}`);
      }
      if (contract.class === "exact-safety-interoperability" && !(typeof contract.authority === "string" && contract.authority.trim())) {
        throw new Error(`E2E exact-safety contract lacks authority for ${script}`);
      }
      if (contract.class === "flexible-presentation-heuristic" && contract.blocking) {
        throw new Error(`E2E flexible-presentation contract cannot block for ${script}`);
      }
    }
    if (entry.stability === "quarantined" && !(typeof entry.quarantineReason === "string" && entry.quarantineReason.trim())) {
      throw new Error(`E2E quarantined contract lacks a reason for ${script}`);
    }
    selected.set(script, entry);
  }
  return selected;
}

function validateEmbeddedFrontend() {
  const index = path.join(root, "dist/index.html");
  if (!fs.existsSync(index)) throw new Error("dist/index.html is missing; run scripts/deploy.sh to build the production frontend");
  const asset = fs.readFileSync(index, "utf8").match(/[A-Za-z0-9_]+-[A-Za-z0-9_-]+\.(?:js|css)/)?.[0];
  if (!asset) throw new Error("could not identify a hashed frontend asset in dist/index.html; run scripts/deploy.sh");
  if (!fs.readFileSync(app).includes(Buffer.from(asset))) {
    throw new Error(`binary does not embed current production frontend ${asset}; run scripts/deploy.sh.`);
  }
  return asset;
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function receiptRemediation(detail) {
  return new Error(`${detail}. Run scripts/deploy.sh or the build receipt helper for the exact app binary before running E2E.`);
}

function loadBuildReceipt() {
  let receipt;
  try {
    receipt = JSON.parse(fs.readFileSync(receiptPath, "utf8"));
  } catch (error) {
    throw receiptRemediation(`could not read build receipt ${receiptPath}: ${error.message}`);
  }
  if (!receipt || Array.isArray(receipt) || typeof receipt !== "object") {
    throw receiptRemediation(`build receipt ${receiptPath} must be a JSON object`);
  }
  return receipt;
}

function validateBuildReceiptInputs() {
  const receipt = loadBuildReceipt();
  const schemaProblems = [];
  if (receipt.schemaVersion !== 1) schemaProblems.push("schemaVersion must be 1");
  if (typeof receipt.sourceRevision !== "string" || !receipt.sourceRevision) schemaProblems.push("sourceRevision must be a non-empty string");
  if (typeof receipt.builtAt !== "string" || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(receipt.builtAt) || Number.isNaN(Date.parse(receipt.builtAt))) {
    schemaProblems.push("builtAt must be an ISO timestamp");
  }
  if (typeof receipt.frontendAsset !== "string" || !receipt.frontendAsset) schemaProblems.push("frontendAsset must be a non-empty string");
  if (!/^[a-f0-9]{64}$/i.test(receipt.appSha256 || "")) schemaProblems.push("appSha256 must be a SHA-256 hex digest");
  if (!/^[a-f0-9]{64}$/i.test(receipt.buildInputDigest || "")) schemaProblems.push("buildInputDigest must be a SHA-256 hex digest");
  if (typeof receipt.buildInputsDirty !== "boolean") schemaProblems.push("buildInputsDirty must be a boolean");
  if (!Array.isArray(receipt.buildInputChanges) || !receipt.buildInputChanges.every((change) => typeof change === "string")) {
    schemaProblems.push("buildInputChanges must be an array of strings");
  }
  if (typeof receipt.buildInputsDirty === "boolean" && Array.isArray(receipt.buildInputChanges) && receipt.buildInputsDirty !== (receipt.buildInputChanges.length > 0)) {
    schemaProblems.push("buildInputsDirty must agree with buildInputChanges");
  }
  if (schemaProblems.length) {
    throw receiptRemediation(`invalid build receipt ${receiptPath}: ${schemaProblems.join(", ")}`);
  }
  if (receipt.buildInputDigest !== buildInputState(root).digest) {
    throw receiptRemediation(`build receipt ${receiptPath} was built from different build inputs than the current checkout`);
  }
  if (receipt.sourceRevision !== checkoutRevision) {
    throw receiptRemediation(`build receipt ${receiptPath} was built from ${receipt.sourceRevision}, not checkout ${checkoutRevision}`);
  }
  if (receipt.buildInputsDirty) {
    throw receiptRemediation(`build receipt ${receiptPath} records dirty binary/frontend inputs`);
  }
  return receipt;
}

function validateBuildReceiptArtifact(receipt, appSha256, frontendAsset) {
  if (receipt.appSha256 !== appSha256) {
    throw receiptRemediation(`build receipt ${receiptPath} hashes a different app binary`);
  }
  if (receipt.frontendAsset !== frontendAsset) {
    throw receiptRemediation(`build receipt ${receiptPath} names frontend asset ${receipt.frontendAsset}, but the current production frontend uses ${frontendAsset}`);
  }
  return {
    kind: "build-receipt",
    testedCommit: receipt.sourceRevision,
    receiptPath,
    sourceRevision: receipt.sourceRevision,
    builtAt: receipt.builtAt,
    frontendAsset: receipt.frontendAsset,
    appSha256: receipt.appSha256,
    buildInputDigest: receipt.buildInputDigest,
    buildInputsDirty: receipt.buildInputsDirty,
    buildInputChanges: receipt.buildInputChanges,
  };
}

function resolveBuildProvenanceInputs() {
  if (fs.existsSync(receiptPath)) return validateBuildReceiptInputs();
  throw receiptRemediation(`build receipt is required at ${receiptPath}`);
}

function failureIsBlocking(status, contractEntry) {
  if (status !== "failed") return false;
  if (e2eMode === "release") {
    return contractEntry.contracts.some((contract) => contract.class !== "flexible-presentation-heuristic");
  }
  return contractEntry.stability !== "quarantined" && contractEntry.contracts.some((contract) => contract.blocking);
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
  });
}

function xmlEscape(value) {
  return String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll('"', "&quot;");
}

function isRetryableDriverTransportFailure(output, errors, timedOut) {
  if (timedOut) return false;
  const combined = `${output}\n${errors}`;
  return /WebDriverError/.test(combined)
    && /\/session/.test(combined)
    && /(UND_ERR_SOCKET|ECONNREFUSED|ECONNRESET|socket hang up|DevToolsActivePort file doesn't exist)/.test(combined);
}

function isRetryableNativeHarnessFailure(id, output, errors, timedOut) {
  if (timedOut || id !== "capture") return false;
  const combined = `${output}\n${errors}`;
  // Hosted Openbox occasionally leaves its active-window property pointing at
  // a frame destroyed during the short single-instance forwarder's teardown.
  // Retry the entire isolated scenario once; the second run must still prove
  // first-show native + DOM focus and save real keyboard input.
  return /BadWindow \(invalid Window parameter\)/.test(combined)
    && /(xdo_get_active_window reported an error|XGetWindowProperty\[_NET_ACTIVE_WINDOW\] failed)/.test(combined);
}

function relativeArtifactPath(file) {
  return path.relative(root, file).split(path.sep).join("/") || ".";
}

function failureClassification(id, output, errors, timedOut) {
  if (isRetryableDriverTransportFailure(output, errors, timedOut) || isRetryableNativeHarnessFailure(id, output, errors, timedOut)) {
    return "infrastructure";
  }
  return "ambiguous";
}

function archiveInfrastructureAttempt(dir, attempt) {
  const archive = path.join(dir, `infrastructure-attempt-${attempt}`);
  fs.mkdirSync(archive, { recursive: true });
  for (const entry of fs.readdirSync(dir)) {
    if (entry.startsWith("infrastructure-attempt-")) continue;
    fs.renameSync(path.join(dir, entry), path.join(archive, entry));
  }
}

async function runScenario([id, script, extraEnv], contractEntry) {
  const started = Date.now();
  const dir = path.join(artifactRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  for (let attempt = 1; attempt <= 2; attempt += 1) {
    const stdout = fs.openSync(path.join(dir, "stdout.log"), "w");
    const stderr = fs.openSync(path.join(dir, "stderr.log"), "w");
    const driverPort = await freePort();
    const nativePort = await freePort();
    const previewPort = await freePort();
    const env = {
      ...baseProcessEnv,
      ...extraEnv,
      TINE_APP: app,
      E2E_ARTIFACT_DIR: dir,
      E2E_DRIVER_PORT: String(driverPort),
      E2E_NATIVE_PORT: String(nativePort),
      E2E_PREVIEW_PORT: String(previewPort),
      TINE_SOURCE_REVISION: buildProvenance.testedCommit || "",
      E2E_LEGACY_NOTES: "0",
      TAURI_DRIVER: process.env.TAURI_DRIVER || (process.platform === "win32" ? "msedgedriver.exe" : "tauri-driver"),
    };
    if (process.platform === "linux") {
      env.WEBKIT_DRIVER = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
    } else if (process.env.WEBKIT_DRIVER) {
      env.WEBKIT_DRIVER = process.env.WEBKIT_DRIVER;
    }
    // Windows WebView2 session creation can fail before WebDriver exposes any
    // application output. Preserve Tine's own startup milestones and panic hook
    // beside the scenario evidence so hosted failures can be classified as an
    // app regression or driver infrastructure rather than guessed from Edge's
    // generic DevToolsActivePort error.
    if (process.platform === "win32") {
      env.TINE_DEBUG = process.env.TINE_DEBUG || "1";
      env.TINE_DEBUG_LOG = process.env.TINE_DEBUG_LOG || path.join(dir, "tine-debug.log");
      env.RUST_BACKTRACE = process.env.RUST_BACKTRACE || "1";
      env.E2E_WEBVIEW_USER_DATA_ROOT = path.join(
        os.tmpdir(),
        `tine-e2e-webview2-${suiteName}-${id}-${process.pid}-${driverPort}`,
      );
    }
    if (id === "og-parity-references") {
      env.E2E_TMP_DIR = process.env.E2E_TMP_DIR
        || path.join(os.tmpdir(), `tine-e2e-${suiteName}-${id}-${process.pid}-${driverPort}`);
    }
    const nativeLinux = process.platform === "linux" && id !== "selection-wrap" && id !== "wayland-app-id";
    // Tauri's Linux single-instance plugin owns a well-known session-bus name.
    // Give each native scenario a private bus so a slow WebKit/Tauri teardown
    // cannot forward the next scenario into the previous app. Processes spawned
    // inside one scenario still share the bus, preserving the multigraph and
    // Quick Capture handoff coverage.
    const command = nativeLinux ? "xvfb-run" : process.execPath;
    const args = nativeLinux
      // Xvfb must wrap the private bus: D-Bus-activated GTK portal services need
      // DISPLAY in the activation environment for auxiliary-window behavior.
      ? ["-a", process.env.DBUS_RUN_SESSION || "dbus-run-session", "--", process.execPath, path.join(root, script)]
      : [path.join(root, script)];
    const child = spawn(command, args, { cwd: root, env, detached: process.platform !== "win32", stdio: ["ignore", stdout, stderr] });
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      try {
        if (process.platform === "win32") child.kill("SIGKILL");
        else process.kill(-child.pid, "SIGKILL");
      } catch {}
    }, timeoutMs);
    const result = await new Promise((resolve) => {
      child.once("error", (error) => resolve({ code: 1, error: String(error) }));
      child.once("exit", (code, signal) => resolve({ code: code ?? 1, signal }));
    });
    clearTimeout(timer);
    if (process.platform === "win32") {
      fs.writeFileSync(
        path.join(dir, "webview2-profile.json"),
        `${JSON.stringify(windowsWebviewProfileSnapshot(env.E2E_WEBVIEW_USER_DATA_ROOT), null, 2)}\n`,
      );
    }
    fs.closeSync(stdout);
    fs.closeSync(stderr);
    const output = fs.readFileSync(path.join(dir, "stdout.log"), "utf8");
    const errors = fs.readFileSync(path.join(dir, "stderr.log"), "utf8");
    const status = result.code === 0 && !timedOut ? "passed" : "failed";
    const retryDriver = isRetryableDriverTransportFailure(output, errors, timedOut);
    const retryNativeHarness = isRetryableNativeHarnessFailure(id, output, errors, timedOut);
    if (status === "failed" && attempt === 1 && (retryDriver || retryNativeHarness)) {
      const reason = retryDriver
        ? "WebDriver session transport failed before app assertions"
        : "hosted X11 active-window state raced a destroyed transient frame";
      process.stdout.write(`RETRY ${id}: ${reason}; retaining attempt 1\n`);
      process.stdout.write(`${output.slice(-1200)}\n${errors.slice(-1200)}\n`);
      archiveInfrastructureAttempt(dir, attempt);
      continue;
    }
    const record = {
      id,
      script,
      status,
      buildProvenance,
      stability: contractEntry.stability,
      contracts: contractEntry.contracts,
      exitCode: result.code,
      signal: result.signal ?? null,
      timedOut,
      attempts: attempt,
      infrastructureRetries: attempt - 1,
      durationMs: Date.now() - started,
      blocking: failureIsBlocking(status, contractEntry),
    };
    if (status === "failed") {
      const failurePath = path.join(dir, "failure.json");
      record.failure = {
        testedCommit: buildProvenance.testedCommit,
        buildProvenance,
        scenario: id,
        script,
        expectedOutcomes: contractEntry.contracts.map((contract) => contract.userOutcome),
        observation: { exitCode: record.exitCode, signal: record.signal, timedOut: record.timedOut },
        classification: failureClassification(id, output, errors, timedOut),
        artifacts: {
          scenario: relativeArtifactPath(dir),
          stdout: relativeArtifactPath(path.join(dir, "stdout.log")),
          stderr: relativeArtifactPath(path.join(dir, "stderr.log")),
          failureCapsule: relativeArtifactPath(failurePath),
        },
      };
      fs.writeFileSync(failurePath, `${JSON.stringify(record.failure, null, 2)}\n`);
    }
    fs.writeFileSync(path.join(dir, "result.json"), JSON.stringify(record, null, 2) + "\n");
    process.stdout.write(`${status === "passed" ? "PASS" : "FAIL"} ${id} (${(record.durationMs / 1000).toFixed(1)}s)\n`);
    if (status === "failed") process.stdout.write(`FAILURE CAPSULE ${JSON.stringify(record.failure)}\n`);
    return record;
  }
  throw new Error(`unreachable scenario retry state for ${id}`);
}

let scenarios = suites[suiteName];
if (only) scenarios = scenarios.filter(([id]) => id === only);
if (!scenarios.length) throw new Error(`no scenario named ${only} in ${suiteName}`);

const receipt = resolveBuildProvenanceInputs();
if (!fs.existsSync(app)) {
  console.error(`production app binary not found: ${app}; build the exact E2E candidate and its receipt first.`);
  process.exit(2);
}
const frontendAsset = validateEmbeddedFrontend();
const appSha256 = sha256(app);
const buildProvenance = validateBuildReceiptArtifact(receipt, appSha256, frontendAsset);
const selectedContracts = loadSelectedContracts(scenarios);
fs.rmSync(artifactRoot, { recursive: true, force: true });
fs.mkdirSync(artifactRoot, { recursive: true });

const results = [];
for (const scenario of scenarios) results.push(await runScenario(scenario, selectedContracts.get(scenario[1])));
const summary = {
  schemaVersion: 1,
  suite: suiteName,
  mode: e2eMode,
  app,
  appSha256,
  buildProvenance,
  platform: `${os.platform()}-${os.arch()}`,
  startedAt: suiteStartedAt,
  passed: results.filter((result) => result.status === "passed").length,
  failed: results.filter((result) => result.status === "failed").length,
  blockingFailed: results.filter((result) => result.blocking).length,
  nonblockingFailed: results.filter((result) => result.status === "failed" && !result.blocking).length,
  quarantinedFailed: results.filter((result) => result.status === "failed" && result.stability === "quarantined").length,
  results,
};
fs.writeFileSync(path.join(artifactRoot, "summary.json"), JSON.stringify(summary, null, 2) + "\n");
const cases = results.map((result) =>
  result.status === "passed"
    ? `<testcase name="${xmlEscape(result.id)}" time="${result.durationMs / 1000}"/>`
    : result.blocking
      ? `<testcase name="${xmlEscape(result.id)}" time="${result.durationMs / 1000}"><failure message="${xmlEscape(`${result.failure.classification}: ${result.failure.expectedOutcomes.join("; ")}`)}">Classification: ${xmlEscape(result.failure.classification)}. Expected user outcome: ${xmlEscape(result.failure.expectedOutcomes.join("; "))}. See ${xmlEscape(result.failure.artifacts.stdout)} and ${xmlEscape(result.failure.artifacts.stderr)}</failure></testcase>`
      : `<testcase name="${xmlEscape(result.id)}" time="${result.durationMs / 1000}"><skipped message="nonblocking E2E observation">Classification: ${xmlEscape(result.failure.classification)}. Expected user outcome: ${xmlEscape(result.failure.expectedOutcomes.join("; "))}. See ${xmlEscape(result.failure.artifacts.stdout)} and ${xmlEscape(result.failure.artifacts.stderr)}</skipped></testcase>`
).join("");
fs.writeFileSync(path.join(artifactRoot, "junit.xml"), `<?xml version="1.0"?><testsuite name="${xmlEscape(suiteName)}" tests="${results.length}" failures="${summary.blockingFailed}" skipped="${summary.failed - summary.blockingFailed}">${cases}</testsuite>\n`);
console.log(`E2E ${suiteName} (${e2eMode}): ${summary.passed} passed, ${summary.blockingFailed} blocking failed, ${summary.nonblockingFailed} nonblocking failed (${summary.quarantinedFailed} quarantined); artifacts ${artifactRoot}`);
process.exit(summary.blockingFailed ? 1 : 0);
