#!/usr/bin/env node

import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import crypto from "node:crypto";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWindowsDevToolsActivePortMirror, windowsWebviewProfileSnapshot } from "./e2e-capabilities.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const suiteName = process.argv[2] ?? "linux-smoke";
const only = process.argv.find((arg) => arg.startsWith("--scenario="))?.slice("--scenario=".length);
const app = path.resolve(process.env.TINE_APP || path.join(root, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine"));
const artifactRoot = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "test-results/e2e", suiteName));
const timeoutMs = Number(process.env.E2E_SCENARIO_TIMEOUT_MS || 180_000);
const suiteStartedAt = new Date().toISOString();
const checkoutRevision = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();

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
    ["rename", "scripts/e2e-rename.mjs", {}],
    ["split-history", "scripts/e2e-split-history.mjs", {}],
    ["alias", "scripts/e2e-alias.mjs", {}],
    ["page-properties", "scripts/e2e-page-properties.mjs", {}],
    ["journal-format", "scripts/e2e-journal-format.mjs", {}],
    ["journal-future-feed", "scripts/e2e-journal-future-feed.mjs", {}],
    ["multigraph", "scripts/e2e-multigraph.mjs", {}],
    ["sheets", "scripts/e2e-sheets.mjs", {}],
    ["selection-wrap", "scripts/e2e-selectwrap.mjs", {}],
    ["tag-autocomplete", "scripts/e2e-tag-autocomplete.mjs", {}],
    ["structured-paste", "scripts/e2e-structured-paste.mjs", {}],
    ["media", "scripts/e2e-media.mjs", {}],
    ["pdf-logseq", "scripts/e2e-pdf-logseq.mjs", {}],
    ["external-assets", "scripts/e2e-external-assets.mjs", {}],
    ["capture", "scripts/e2e-capture.mjs", { E2E_WINDOW_MANAGER: "openbox" }],
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
    ["pdf-logseq", "scripts/e2e-pdf-logseq.mjs", {}],
    ["print-security", "scripts/e2e-print-security.mjs", {}],
    ["windows-core", "scripts/e2e-windows-smoke.mjs", {}],
    ["page-trailing-block", "scripts/e2e-page-trailing-block.mjs", {}],
  ],
};

if (!suites[suiteName]) {
  console.error(`unknown suite ${suiteName}; choose ${Object.keys(suites).join(", ")}`);
  process.exit(2);
}

if (!fs.existsSync(app)) {
  console.error(`production app binary not found: ${app}`);
  process.exit(2);
}

function validateEmbeddedFrontend() {
  const index = path.join(root, "dist/index.html");
  if (!fs.existsSync(index)) throw new Error("dist/index.html is missing; build the production frontend first");
  const asset = fs.readFileSync(index, "utf8").match(/[A-Za-z0-9_]+-[A-Za-z0-9_-]+\.(?:js|css)/)?.[0];
  if (!asset) throw new Error("could not identify a hashed frontend asset in dist/index.html");
  if (!fs.readFileSync(app).includes(Buffer.from(asset))) {
    throw new Error(`binary does not embed current production frontend ${asset}; build with custom-protocol`);
  }
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

function archiveInfrastructureAttempt(dir, attempt) {
  const archive = path.join(dir, `infrastructure-attempt-${attempt}`);
  fs.mkdirSync(archive, { recursive: true });
  for (const entry of fs.readdirSync(dir)) {
    if (entry.startsWith("infrastructure-attempt-")) continue;
    fs.renameSync(path.join(dir, entry), path.join(archive, entry));
  }
}

async function runScenario([id, script, extraEnv]) {
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
      TINE_SOURCE_REVISION: process.env.TINE_SOURCE_REVISION || checkoutRevision,
      E2E_LEGACY_NOTES: "0",
      TAURI_DRIVER: process.env.TAURI_DRIVER || "tauri-driver",
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
    const stopActivePortMirror = startWindowsDevToolsActivePortMirror(env.E2E_WEBVIEW_USER_DATA_ROOT);
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
    stopActivePortMirror();
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
      exitCode: result.code,
      signal: result.signal ?? null,
      timedOut,
      attempts: attempt,
      infrastructureRetries: attempt - 1,
      durationMs: Date.now() - started,
    };
    fs.writeFileSync(path.join(dir, "result.json"), JSON.stringify(record, null, 2) + "\n");
    process.stdout.write(`${status === "passed" ? "PASS" : "FAIL"} ${id} (${(record.durationMs / 1000).toFixed(1)}s)\n`);
    if (status === "failed") process.stdout.write(`${output.slice(-2000)}\n${errors.slice(-2000)}\n`);
    return record;
  }
  throw new Error(`unreachable scenario retry state for ${id}`);
}

validateEmbeddedFrontend();
fs.rmSync(artifactRoot, { recursive: true, force: true });
fs.mkdirSync(artifactRoot, { recursive: true });
let scenarios = suites[suiteName];
if (only) scenarios = scenarios.filter(([id]) => id === only);
if (!scenarios.length) throw new Error(`no scenario named ${only} in ${suiteName}`);

const results = [];
for (const scenario of scenarios) results.push(await runScenario(scenario));
const summary = {
  schemaVersion: 1,
  suite: suiteName,
  app,
  appSha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  platform: `${os.platform()}-${os.arch()}`,
  startedAt: suiteStartedAt,
  passed: results.filter((result) => result.status === "passed").length,
  failed: results.filter((result) => result.status === "failed").length,
  results,
};
fs.writeFileSync(path.join(artifactRoot, "summary.json"), JSON.stringify(summary, null, 2) + "\n");
const cases = results.map((result) =>
  result.status === "passed"
    ? `<testcase name="${xmlEscape(result.id)}" time="${result.durationMs / 1000}"/>`
    : `<testcase name="${xmlEscape(result.id)}" time="${result.durationMs / 1000}"><failure message="scenario failed">See ${xmlEscape(result.id)}/stdout.log and stderr.log</failure></testcase>`
).join("");
fs.writeFileSync(path.join(artifactRoot, "junit.xml"), `<?xml version="1.0"?><testsuite name="${xmlEscape(suiteName)}" tests="${results.length}" failures="${summary.failed}">${cases}</testsuite>\n`);
console.log(`E2E ${suiteName}: ${summary.passed} passed, ${summary.failed} failed; artifacts ${artifactRoot}`);
process.exit(summary.failed ? 1 : 0);
