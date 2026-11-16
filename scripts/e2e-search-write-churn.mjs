// GH #907: literal Search-tab navigation during guarded CLI write streams.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import net from "node:net";
import { spawn, execFile, execFileSync } from "node:child_process";
import { promisify } from "node:util";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/fix-beta-search-churn-907");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const page = i => `Churn Result ${String(i).padStart(3, "0")}`;
const uuid = i => `90700000-0000-4000-8000-${String(i).padStart(12, "0")}`;
for (let i = 0; i < 75; i++) fs.writeFileSync(path.join(graph, `pages/${page(i)}.md`),
  `- churnneedle result ${i}\n  id:: ${uuid(i)}\n`);
fs.writeFileSync(path.join(graph, "pages/Writer.md"), "- Unrelated writer\n");
fs.writeFileSync(path.join(graph, "pages/Churn Home.md"), "- Search while writers are active\n");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = await freePort(), nativePort = await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn("tauri-driver", webdriverServerArgs(port, nativePort, "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), writes: [], observations: {} };
const run = promisify(execFile);
async function cli(args) {
  const result = await run(app, [args[0], "--graph", graph, "--json", ...args.slice(1)], { env: fixture.env, timeout: 15000 });
  proof.writes.push({ args, result: JSON.parse(result.stdout) });
}
async function stream(name, count = 12, matching = false) {
  for (let i = 0; i < count; i++) {
    await cli(["add", "--page", name, matching ? `- churnneedle live stream ${i}` : `- background prose ${i}`]);
    await sleep(150);
    if (matching && i === 5) {
      proof.observations.midstream = await state();
      assert.ok(proof.observations.midstream.rows > 75, "continuous matching writes postponed all live membership until the writer stopped");
    }
  }
}
let browser, stage = "startup";
const shot = name => execFileSync("import", ["-window", "root", path.join(out, name)], { env: fixture.env });
const state = () => browser.execute(() => {
  const probe = window.__searchChurn;
  const rows = [...document.querySelectorAll(".query-workspace [data-query-result]")];
  const witness = probe?.witness;
  const rect = witness?.getBoundingClientRect();
  return { rows: rows.length, retained: probe?.rows.filter(row => row.isConnected).length,
    removed: probe?.removed, calls: probe?.calls, witnessConnected: witness?.isConnected,
    ipc: probe?.ipc, summaries: probe?.summaries,
    scrollTop: document.querySelector(".main-content")?.scrollTop,
    witnessTop: rect?.top, underPointer: probe?.point && document.elementFromPoint(probe.point.x, probe.point.y)?.closest("[data-query-result]") === witness,
    text: document.querySelector(".query-workspace-status")?.textContent };
});
try {
  await sleep(300);
  assert.equal(driver.exitCode, null);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app) });
  await browser.waitUntil(async () => {
    await ensureMainWindow(browser);
    return browser.execute(() => Boolean(document.querySelector(".app-container")));
  }, { timeout: 60000, timeoutMsg: "the application window did not finish startup" });
  await assertAppLease(fixture);
  await openPageByName(browser, "Churn Home");
  // Establish the host's first full publication before measuring precise keys.
  await cli(["add", "--page", "Writer", "- publication baseline"]);
  await sleep(1000);
  await browser.execute(() => {
    const probe = window.__searchChurn = { calls: 0, ipc: [], summaries: [] };
    const original = window.fetch;
    window.fetch = function(input, init) {
      const url = new URL(typeof input === "string" ? input : input.url);
      const command = decodeURIComponent(url.pathname.slice(1));
      if ((url.protocol === "ipc:" || url.hostname === "ipc.localhost") && ["run_graph_search", "register_query_subscription"].includes(command)) {
        if (command === "run_graph_search") probe.calls++;
        if (probe.ipc.length < 100) probe.ipc.push({ command, args: typeof init?.body === "string" ? JSON.parse(init.body) : null });
      }
      return original.call(this, input, init);
    };
    const handler = window.__TAURI_INTERNALS__.transformCallback(event => {
      if (probe.summaries.length < 100) probe.summaries.push(event.payload?.answers?.summary ?? event.payload);
    });
    void window.__TAURI_INTERNALS__.invoke("plugin:event|listen", { event: "graph-changed", target: { kind: "Any" }, handler });
  });
  stage = "open Search tab";
  await browser.keys(["Control", "k"]); await browser.releaseActions();
  const input = await browser.$(".switcher-input"); await input.waitForDisplayed();
  await input.setValue("churnneedle");
  const open = await browser.$("[data-open-search-tab]"); await open.waitForDisplayed(); await open.click();
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".query-workspace [data-query-result]").length === 75),
    { timeout: 15000, timeoutMsg: "Search tab did not show its scrollable answer" });
  proof.observations.initialSearchCalls = await browser.execute(() => window.__searchChurn.calls);
  assert.ok(proof.observations.initialSearchCalls > 0, "IPC observer did not see the initial real search");
  await browser.execute(() => {
    const probe = window.__searchChurn;
    Object.assign(probe, { rows: [...document.querySelectorAll(".query-workspace [data-query-result]")], removed: 0, calls: 0 });
    probe.witness = probe.rows.find(row => row.textContent.includes("churnneedle result 55"));
    probe.observer = new MutationObserver(records => {
      for (const record of records) for (const removed of record.removedNodes)
        if (probe.rows.some(row => !row.isConnected && (removed === row || removed.contains(row)))) probe.removed++;
    });
    probe.observer.observe(document.querySelector(".query-workspace"), { childList: true, subtree: true });
  });
  const lower = await browser.$('//button[@data-query-result][contains(.,"churnneedle result 55")]');
  await lower.scrollIntoView({ block: "center" });
  stage = "unrelated CLI writes while scrolling";
  const unrelated = stream("Writer");
  await browser.action("wheel").scroll({ x: 800, y: 400, deltaX: 0, deltaY: -300, duration: 0 }).perform();
  await lower.scrollIntoView({ block: "center" });
  await lower.moveTo();
  await unrelated;
  await sleep(1000);
  proof.observations.unrelated = await state();
  shot("unrelated-writes-scrolled.png");
  assert.equal(proof.observations.unrelated.retained, 75, "unrelated writes replaced result rows");
  assert.equal(proof.observations.unrelated.calls, 0, "unrelated writes reran graph search");
  stage = "related CLI writes with pointer on lower result";
  await browser.execute(() => {
    const probe = window.__searchChurn, rect = probe.witness.getBoundingClientRect();
    probe.point = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  });
  proof.observations.beforeRelated = await state();
  await stream(page(0), 12, true);
  await cli(["edit", uuid(0), `churnneedle CHURNHEIGHT ${"expanded visible context for the first result ".repeat(20)}`]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".query-workspace")?.textContent.includes("CHURNHEIGHT")),
    { timeout: 15000, timeoutMsg: "matching result text did not refresh" });
  proof.observations.resized = await state();
  assert.equal(proof.observations.resized.underPointer, true, "a changed result above moved the lower result away from the pointer");
  // New matching membership ABOVE the witness must update live and leave the
  // same result under the stationary pointer (also checks WebKit scroll anchor).
  await cli(["add", "--before", uuid(0), "- churnneedle newly inserted result"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".query-workspace [data-query-result]").length === 88),
    { timeout: 15000, timeoutMsg: "related write did not update live membership" });
  await sleep(300);
  proof.observations.related = await state();
  shot("related-writes-anchored.png");
  assert.equal(proof.observations.related.retained, 75, "related writes remounted unchanged rows");
  assert.equal(proof.observations.related.removed, 0, "write refresh replaced a result list or row");
  assert.equal(proof.observations.related.underPointer, true, "the lower result moved away from the pointer");
  stage = "click lower result during active CLI writes";
  const more = stream("Writer", 4);
  const point = await browser.execute(() => window.__searchChurn.point);
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, x: Math.round(point.x), y: Math.round(point.y) },
    { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  await browser.waitUntil(() => browser.execute((name) =>
    document.querySelector(".main-content .zoom-breadcrumb .crumb-page")?.textContent.trim() === name
      && document.querySelector(".main-content .zoomed-block")?.textContent.includes("churnneedle result 55"), page(55)),
    { timeout: 15000, timeoutMsg: "the lower result did not open its physical source page" });
  await more;
  proof.observations.destination = await browser.execute(() => ({
    page: document.querySelector(".main-content .zoom-breadcrumb .crumb-page")?.textContent.trim(),
    block: document.querySelector(".main-content .zoomed-block")?.textContent.trim(),
  }));
  shot("lower-result-opened.png");
  fixture.verify();
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL"; proof.stage = stage; proof.observed = String(error.stack);
  proof.expected = "Search stays scrollable, preserves rows and pointer anchor, and opens the lower result during CLI writes";
  proof.classification = error.code === "ERR_ASSERTION" ? "product" : "ambiguous; inspect screenshot and driver log";
  try {
    proof.observations.failure = await state();
    proof.document = await browser.execute(() => ({ url: location.href, text: document.body.innerText }));
    shot("failure.png");
  } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`${proof.outcome}: ${out}`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fixture.close(); fs.closeSync(log);
}
