#!/usr/bin/env node

import { spawn } from "node:child_process";
import fs from "node:fs";
import crypto from "node:crypto";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const suiteName = process.argv[2] ?? "linux-smoke";
const only = process.argv.find((arg) => arg.startsWith("--scenario="))?.slice("--scenario=".length);
const app = path.resolve(process.env.TINE_APP || path.join(root, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine"));
const artifactRoot = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "test-results/e2e", suiteName));
const timeoutMs = Number(process.env.E2E_SCENARIO_TIMEOUT_MS || 180_000);
const suiteStartedAt = new Date().toISOString();

const suites = {
  "linux-smoke": [
    ["caret-agenda", "scripts/e2e-caret.mjs", { CARET_MODE: "agenda", CARET_LABEL: "runner" }],
    ["multigraph", "scripts/e2e-multigraph.mjs", {}],
    ["sheets", "scripts/e2e-sheets.mjs", {}],
  ],
  "linux-release": [
    ["caret-agenda", "scripts/e2e-caret.mjs", { CARET_MODE: "agenda", CARET_LABEL: "runner" }],
    ["caret-page", "scripts/e2e-caret.mjs", { CARET_MODE: "page", CARET_LABEL: "runner" }],
    ["click-caret", "scripts/e2e-clickcaret-repro.mjs", {}],
    ["block-select", "scripts/e2e-blockselect.mjs", {}],
    ["rename", "scripts/e2e-rename.mjs", {}],
    ["journal-format", "scripts/e2e-journal-format.mjs", {}],
    ["multigraph", "scripts/e2e-multigraph.mjs", {}],
    ["sheets", "scripts/e2e-sheets.mjs", {}],
    ["selection-wrap", "scripts/e2e-selectwrap.mjs", {}],
    ["structured-paste", "scripts/e2e-structured-paste.mjs", {}],
    ["media", "scripts/e2e-media.mjs", {}],
    ["capture", "scripts/e2e-capture.mjs", { E2E_WINDOW_MANAGER: "openbox" }],
    ["page-file-actions", "scripts/e2e-page-file-actions.mjs", {}],
    ["block-embed", "scripts/e2e-block-embed.mjs", {}],
    ["sidebar-sections", "scripts/e2e-sidebar-sections.mjs", {}],
    ["right-sidebar-collapse", "scripts/e2e-right-sidebar-collapse.mjs", {}],
    ["tab-overflow", "scripts/e2e-tab-overflow.mjs", {}],
  ],
  "windows-smoke": [
    ["windows-core", "scripts/e2e-windows-smoke.mjs", {}],
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

async function runScenario([id, script, extraEnv]) {
  const started = Date.now();
  const dir = path.join(artifactRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  const stdout = fs.openSync(path.join(dir, "stdout.log"), "w");
  const stderr = fs.openSync(path.join(dir, "stderr.log"), "w");
  const driverPort = await freePort();
  const nativePort = await freePort();
  const previewPort = await freePort();
  const env = {
    ...process.env,
    ...extraEnv,
    TINE_APP: app,
    E2E_ARTIFACT_DIR: dir,
    E2E_DRIVER_PORT: String(driverPort),
    E2E_NATIVE_PORT: String(nativePort),
    E2E_PREVIEW_PORT: String(previewPort),
    E2E_LEGACY_NOTES: "0",
    TAURI_DRIVER: process.env.TAURI_DRIVER || "tauri-driver",
    WEBKIT_DRIVER: process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
  };
  const nativeLinux = process.platform === "linux" && id !== "selection-wrap";
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
  fs.closeSync(stdout);
  fs.closeSync(stderr);
  const output = fs.readFileSync(path.join(dir, "stdout.log"), "utf8");
  const errors = fs.readFileSync(path.join(dir, "stderr.log"), "utf8");
  const status = result.code === 0 && !timedOut ? "passed" : "failed";
  const record = { id, script, status, exitCode: result.code, signal: result.signal ?? null, timedOut, durationMs: Date.now() - started };
  fs.writeFileSync(path.join(dir, "result.json"), JSON.stringify(record, null, 2) + "\n");
  process.stdout.write(`${status === "passed" ? "PASS" : "FAIL"} ${id} (${(record.durationMs / 1000).toFixed(1)}s)\n`);
  if (status === "failed") process.stdout.write(`${output.slice(-2000)}\n${errors.slice(-2000)}\n`);
  return record;
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
