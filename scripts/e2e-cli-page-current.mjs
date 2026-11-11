// D-20261110-CP / discussion 925: scriptable focused-page lookup in the real app.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const run = path.resolve(process.env.E2E_TMP_DIR || `target/artifacts/cli-page-current-925/native-${process.pid}`);
assert(!fs.existsSync(run), "use fresh scratch paths");
const graph = path.join(run, "graph");
for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
const uuid = "92500000-0000-4000-8000-000000000001";
const file = path.join(graph, "pages/Current.md");
const bytes = `- current block\n  id:: ${uuid}\n`;
fs.writeFileSync(file, bytes);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:default-home {:page "Current"}}\n');
const fixture = installAccountHome(run);
const env = { ...process.env, HOME: path.join(run, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  TAURI_WEBVIEW_AUTOMATION: "true", WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
fs.mkdirSync(env.HOME, { recursive: true });
const port = Number(process.env.E2E_DRIVER_PORT || 4925);
const native = Number(process.env.E2E_NATIVE_PORT || 4926);
const commit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const transcript = [];
let driver, browser;
function current(expected = 0, json = true) {
  return new Promise((resolve, reject) => {
    const started = performance.now();
    const child = spawn(app, ["page", "current", "--graph", graph, ...(json ? ["--json"] : [])], { env, timeout: 10_000 });
    let out = "", err = "";
    child.stdout.on("data", (value) => out += value);
    child.stderr.on("data", (value) => err += value);
    child.on("error", reject);
    child.on("exit", (code) => {
      try {
        assert.equal(code, expected, `${out}\n${err}`);
        const reply = json ? JSON.parse(out) : out;
        transcript.push({ code, reply, milliseconds: performance.now() - started });
        resolve(reply);
      } catch (error) { reject(error); }
    });
  });
}
try {
  assert.equal((await current(3)).reason, "no-owner-endpoint");
  assert(!fs.existsSync(path.join(graph, "logseq/tine-graph-id")));
  const log = fs.openSync(path.join(run, "driver.log"), "w");
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, native, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, detached: true, stdio: ["ignore", log, log] });
  fs.closeSync(log);
  // HTTP readiness, bounded to this driver's endpoint; no host process polling.
  for (let attempts = 0; ; attempts++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    assert(attempts < 100 && driver.exitCode === null, "driver did not start");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000, capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await openPageByName(browser, "Current");
  await assertAppLease(fixture);
  await browser.$(".page-title").click();
  assert.deepEqual(await current(), { schema: "tine.cli/1", kind: "current-page", name: "Current", url: "tine://page/Current", block_url: null });
  assert.equal(await current(0, false), "Current\ntine://page/Current\n");
  // data-block-id is the session id, not the `id::` uuid; the page has exactly one block.
  await browser.$(".ls-block .block-content-wrapper").click();
  await browser.waitUntil(() => browser.execute(() => !!document.activeElement?.closest("[data-block-id]")), { timeout: 10_000 });
  assert.equal((await current()).block_url, `tine://block/${uuid}`);
  await browser.$("button=All pages").click();
  assert.equal((await current(3)).reason, "not-page");
  assert.equal(fs.readFileSync(file, "utf8"), bytes);
  assert(!fs.existsSync(path.join(graph, "logseq/tine-graph-id")));
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, passed: true, transcript }, null, 2));
  console.log("PASS: current page, focused block, non-page view, text/JSON, and unchanged graph bytes");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ commit, journey: "CLI focused page", expected: "focused page/block or typed absence without graph I/O", observed: String(error), classification: "ambiguous", transcript }, null, 2));
  console.error(error);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (driver) { try { process.kill(-driver.pid, "SIGTERM"); } catch {} }
}
