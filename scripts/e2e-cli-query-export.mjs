// #588: the installed CLI publishes each query source through the real app owner.
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { remote } from "webdriverio";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();
const root = path.resolve(import.meta.dirname, "..");
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/export-query-588");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
const uuid = "82400000-0000-4000-8000-000000000588";
const source = `alias:: Query shortcut\n- literal\n  note:: {{query (task DONE)}}\n- parent\n  - {{query (task TODO)}}\n    id:: ${uuid}\n- {{query (task DONE)}}\n`;
fs.writeFileSync(path.join(graph, "pages/Queries.md"), source);
const selected = "- TODO exported sentinel\n";
fs.writeFileSync(path.join(graph, "pages/Selected.md"), selected);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:default-home {:page "Queries"} :tine/guide-announced? true}\n');
const runtime = fs.mkdtempSync(path.join(os.tmpdir(), "tq588-"));
const fixture = installAccountHome(runtime, run);
const env = { ...process.env, HOME: path.join(runtime, "home"), TINE_GRAPH: graph, TINE_QUICK_SHARE: "0",
  XDG_DATA_HOME: path.join(runtime, "data"), XDG_CONFIG_HOME: path.join(runtime, "config"),
  XDG_STATE_HOME: path.join(runtime, "state"), XDG_CACHE_HOME: path.join(runtime, "cache"),
  XDG_RUNTIME_DIR: path.join(runtime, "runtime"),
  SOURCE_DATE_EPOCH: "0", WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
fs.mkdirSync(env.HOME, { recursive: true });
fs.mkdirSync(env.XDG_RUNTIME_DIR, { mode: 0o700 });
const port = Number(process.env.E2E_DRIVER_PORT || 4868);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
fs.closeSync(log);
const proof = { commit: execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "CLI query export: UUID, inline and page; first export, refusal, force recovery",
  expected: "All sources export Selected, preserve source bytes and recover replaced output", steps: [] };
let browser;
function cli(args, expected = 0) {
  return new Promise((resolve, reject) => {
    const child = spawn(app, args, { env, timeout: 60_000 });
    let stdout = "", stderr = "";
    child.stdout.on("data", value => stdout += value);
    child.stderr.on("data", value => stderr += value);
    child.on("error", reject);
    child.on("exit", code => {
      try { assert.equal(code, expected, `${args.join(" ")}: ${stderr}`); resolve({ stdout, stderr }); }
      catch (error) { reject(error); }
    });
  });
}
try {
  const deadline = Date.now() + 15_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    assert(Date.now() < deadline, "driver startup failed");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 60_000, capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await browser.$(".page-title").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Queries");
  await assertAppLease(fixture);
  const status = await cli(["status", "--graph", graph, "--json"]);
  assert.equal(JSON.parse(status.stdout).owner, "app");
  for (const [label, selector] of [["uuid", [uuid]], ["inline", ["--query", "(task TODO)"]], ["page", ["--page", "qUeRy ShOrTcUt"]]]) {
    const name = `Review ${label}`;
    const args = ["export", "query", graph, ...selector, "--name", name];
    const output = path.join(graph, "published-queries", `review-${label}`);
    assert((await cli(args)).stdout.includes("Published 1 pages"));
    const snapshot = fs.readFileSync(path.join(output, "app/snapshot.json"), "utf8");
    assert(snapshot.includes("exported sentinel"));
    const refused = await cli(args, 1);
    assert(refused.stderr.includes("--force"));
    assert.equal(fs.readFileSync(path.join(output, "app/snapshot.json"), "utf8"), snapshot);
    const recovery = path.join(graph, "logseq/.tine-trash/conflicts");
    const before = new Set(fs.existsSync(recovery) ? fs.readdirSync(recovery, { recursive: true }) : []);
    const forced = await cli([...args, "--force"]);
    assert(forced.stdout.includes("Published 1 pages"));
    const retained = fs.readdirSync(recovery, { recursive: true })
      .find(file => !before.has(file) && String(file).endsWith("snapshot.json"));
    assert(retained, `${label}: previous snapshot retained`);
    assert.equal(fs.readFileSync(path.join(recovery, retained), "utf8"), snapshot);
    assert.equal(fs.readFileSync(path.join(graph, "pages/Queries.md"), "utf8"), source);
    assert.equal(fs.readFileSync(path.join(graph, "pages/Selected.md"), "utf8"), selected);
    proof.steps.push(`${label}: first export, collision refused, force retained previous publication; source unchanged`);
  }
  proof.outcome = "passed";
} catch (error) {
  proof.outcome = "failed"; proof.observed = String(error); proof.classification = "ambiguous";
  if (browser) await browser.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  throw error;
} finally {
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  if (browser) await browser.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  if (driver.exitCode === null && driver.signalCode === null) await new Promise(resolve => driver.once("exit", resolve));
  console.log(path.join(run, "proof.json"));
}
