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
const otherGraph = path.join(run, "other-graph");
for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
const uuid = "92500000-0000-4000-8000-000000000001";
const file = path.join(graph, "pages/Current.md");
const bytes = `- current block\n  id:: ${uuid}\n- plain block\n`;
fs.writeFileSync(file, bytes);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:default-home {:page "Current"}}\n');
for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(otherGraph, sub), { recursive: true });
fs.writeFileSync(path.join(otherGraph, "pages/Current.md"), bytes);
fs.writeFileSync(path.join(otherGraph, "logseq/config.edn"), '{:default-home {:page "Current"}}\n');
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
function current(expected = 0, json = true, root = graph) {
  return new Promise((resolve, reject) => {
    const started = performance.now();
    const child = spawn(app, ["page", "current", "--graph", root, ...(json ? ["--json"] : [])], { env, timeout: 10_000 });
    let out = "", err = "";
    child.stdout.on("data", (value) => out += value);
    child.stderr.on("data", (value) => err += value);
    child.on("error", reject);
    child.on("exit", (code) => {
      try {
        assert.equal(code, expected, `${out}\n${err}`);
        if (expected === 1) assert.equal(out, "", "identity failure publishes no page payload");
        const reply = json ? JSON.parse(expected === 1 ? err : out) : out;
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
  assert(!fs.existsSync(path.join(graph, "logseq/tine-graph-id")), "opening a page does not create its graph identity");
  const first = await current();
  const identityPath = path.join(graph, "logseq/tine-graph-id");
  const identityBytes = fs.readFileSync(identityPath, "utf8");
  assert.equal(identityBytes.length, 37);
  const graphId = identityBytes.trim();
  const url = `tine://page/Current?graph=${graphId}`;
  assert.deepEqual(first, { schema: "tine.cli/1", kind: "current-page", name: "Current", url, block_url: null });
  const identityStamp = fs.statSync(identityPath, { bigint: true }).mtimeNs;
  assert.equal(await current(0, false), `Current\n${url}\n`);
  assert.equal(fs.readFileSync(identityPath, "utf8"), identityBytes);
  assert.equal(fs.statSync(identityPath, { bigint: true }).mtimeNs, identityStamp, "second request reuses the file");
  // data-block-id is the session id, not the `id::` uuid. Two blocks: the first authored, the second plain.
  const focusBlock = async (index) => {
    await (await browser.$$(".ls-block .block-content-wrapper"))[index].click();
    await browser.waitUntil(() => browser.execute(() => !!document.activeElement?.closest("[data-block-id]")), { timeout: 10_000 });
  };
  await focusBlock(0);
  // PC-3: one frontend answer per request, and the end-to-end budget.
  await browser.execute(() => {
    // invoke is frozen. Count at the transport, as e2e-seen-changes does.
    if (window.__pageCurrentCalls) return;
    window.__pageCurrentCalls = [];
    const record = (command) => { if (command) window.__pageCurrentCalls.push(command); };
    const fetch = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const url = typeof input === "string" ? input : input?.url ?? String(input);
      if (/ipc/.test(url)) record(decodeURIComponent(new URL(url).pathname.split("/").pop() ?? ""));
      return fetch(input, init);
    };
    const handler = window.webkit?.messageHandlers?.ipc;
    if (handler?.postMessage) {
      const post = handler.postMessage.bind(handler);
      handler.postMessage = (message) => {
        try { record(JSON.parse(typeof message === "string" ? message : "{}").cmd); } catch {}
        return post(message);
      };
    }
  });
  const times = [];
  let answers = 0;
  for (let attempt = 0; attempt < 5; attempt++) {
    await browser.execute(() => { window.__pageCurrentCalls.length = 0; });
    assert.equal((await current()).block_url, `tine://block/${uuid}`);
    times.push(transcript.at(-1).milliseconds);
    const calls = await browser.execute(() => [...window.__pageCurrentCalls]);
    answers = calls.filter((command) => command === "owner_current_page_answer").length;
    assert.equal(answers, 1, `one frontend answer per request: ${calls}`);
    assert.equal(calls.filter((command) => command === "graph_link_identity").length, 0, "identity is carried by the owner question");
  }
  times.sort((a, b) => a - b);
  assert(times[2] < 1500, `median ${times[2]} ms over the 1500 ms budget (lower bound: one socket exchange and one webview answer)`);
  // A block without an authored id has no durable address: no link, and nothing written to make one.
  await focusBlock(1);
  assert.equal((await current()).block_url, null);
  // Two open graphs with the same page title must yield distinct durable addresses.
  const main = await browser.getWindowHandle();
  const before = await browser.getWindowHandles();
  execFileSync(app, [otherGraph], { env: { ...env, TINE_GRAPH: "" }, timeout: 10_000 });
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length > before.length, { timeout: 30_000 });
  const peer = (await browser.getWindowHandles()).find((handle) => !before.includes(handle));
  await browser.switchToWindow(peer);
  await openPageByName(browser, "Current");
  assert(!fs.existsSync(path.join(otherGraph, "logseq/tine-graph-id")));
  const second = await current(0, true, otherGraph);
  const otherId = fs.readFileSync(path.join(otherGraph, "logseq/tine-graph-id"), "utf8").trim();
  assert.notEqual(otherId, graphId);
  assert.equal(second.url, `tine://page/Current?graph=${otherId}`);
  assert.equal((await current()).url, url, "requesting the first graph still names it while the second is focused");
  await browser.switchToWindow(main);
  await browser.$("button=All pages").click();
  assert.equal((await current(3)).reason, "not-page");
  assert.equal(fs.readFileSync(file, "utf8"), bytes);
  assert.equal(fs.readFileSync(identityPath, "utf8"), identityBytes);
  assert.equal(fs.statSync(identityPath, { bigint: true }).mtimeNs, identityStamp);
  assert.equal(fs.readFileSync(path.join(otherGraph, "pages/Current.md"), "utf8"), bytes);
  // A failed identity read reports its cause and preserves the established bytes.
  fs.writeFileSync(identityPath, "torn");
  const failed = await current(1);
  assert.equal(failed.error.code, "internal");
  assert.equal(failed.error.detail.diag, "cli-write-failed");
  assert.match(failed.error.message, /Graph link identity is invalid/);
  assert.equal(fs.readFileSync(identityPath, "utf8"), "torn");
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, passed: true, transcript }, null, 2));
  console.log("PASS: two graph-qualified page links, identity created once and reused, authored/plain focused block, one answer and budget, non-page view, text/JSON, and unchanged note bytes");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ commit, journey: "CLI focused page", expected: "graph-qualified page/block or typed absence, one guarded identity create and unchanged notes", observed: String(error), classification: "ambiguous", transcript }, null, 2));
  console.error(error);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (driver) { try { process.kill(-driver.pid, "SIGTERM"); } catch {} }
}
