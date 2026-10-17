// GH #551 / #387: real parser, builder removal and Search selection workflow.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

fs.mkdirSync(path.resolve("artifacts/sw-queries"), { recursive: true });
const out = fs.mkdtempSync(path.resolve("artifacts/sw-queries/native-"));
const graph = path.join(out, "graph");
for (const dir of ["graph/pages", "graph/journals", "graph/logseq", "data", "config", "cache"]) fs.mkdirSync(path.join(out, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:shortcuts {:go/search-tab "mod+alt+f"}}\n');
fs.writeFileSync(path.join(graph, "pages/Sweep.md"), '- {{query (task TODO) {:title "Remove condition"}}}\n- {{query (and) {:title "Authored blank"}}}\n');
fs.writeFileSync(path.join(graph, "pages/Needle.md"), '- TODO Needle task\n');
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4918);
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const testedCommit = execFileSync("rtk", ["git", "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup";
const screenshot = name => execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, name)], { env, timeout: 10000 });
const query = title => browser.execute(wanted => [...document.querySelectorAll(".query-block")].find(q => q.querySelector(".query-title")?.textContent.trim() === wanted)?.textContent, title);
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "sw-queries") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Sweep");
  journey = "authored empty query";
  await browser.waitUntil(async () => (await query("Authored blank"))?.includes("Add a condition to see results."), { timeout: 15000 });
  journey = "remove last builder condition";
  await browser.waitUntil(async () => (await query("Remove condition"))?.includes("Needle task"), { timeout: 15000 });
  await browser.execute(() => {
    const q = [...document.querySelectorAll(".query-block")].find(q => q.querySelector(".query-title")?.textContent.trim() === "Remove condition");
    q.querySelector(".qc-actions > summary").click();
  });
  await browser.waitUntil(() => browser.execute(() => {
    const q = [...document.querySelectorAll(".query-block")].find(q => q.querySelector(".query-title")?.textContent.trim() === "Remove condition");
    const menu = document.getElementById(q.querySelector("summary").getAttribute("aria-controls"));
    const edit = [...(menu?.querySelectorAll("button") ?? [])].find(b => b.textContent === "Edit");
    if (!edit) return false;
    edit.click(); return true;
  }), { timeout: 10000, timeoutMsg: "query action portal never exposed Edit" });
  await browser.$(".qc-panel .qs-row-remove").waitForExist({ timeout: 10000 });
  await browser.$(".qc-panel .qs-row-remove").click();
  await browser.waitUntil(async () => (await query("Remove condition"))?.includes("Add a condition to see results."), { timeout: 15000 });
  await browser.execute(() => document.querySelector(".qc-panel .qc-done")?.click());
  journey = "Search selection toggle, actions and result kinds";
  await browser.keys(["Control", "Alt", "f"]);
  const input = await browser.$(".query-workspace-source");
  await input.waitForDisplayed({ timeout: 10000 });
  await input.setValue("Needle");
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".query-workspace button[data-query-result]").length >= 2), { timeout: 15000 });
  const toggle = await browser.$(".query-results-selection > button");
  await toggle.click();
  assert.equal(await toggle.getAttribute("aria-pressed"), "true");
  const selection = await browser.execute(() => {
    const root = document.querySelector(".query-results-selection");
    const first = root.querySelector(".query-copy-row");
    return { kinds: [...root.querySelectorAll(".switcher-kind")].map(el => el.textContent),
      actionsAbove: [...root.querySelectorAll(".query-selection-actions button")].every(b => b.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING) };
  });
  assert.ok(selection.kinds.includes("page") && selection.kinds.includes("block"));
  assert.ok(selection.actionsAbove);
  screenshot("selection.png");
  await toggle.click();
  assert.equal(await browser.$(".query-copy-row").isExisting(), false);
  await toggle.click();
  await browser.execute(() => [...document.querySelectorAll(".query-selection-actions button")].find(b => b.textContent === "Done").click());
  assert.equal(await toggle.getAttribute("aria-pressed"), "false");
  screenshot("accepted.png");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify({ testedCommit, workingDiff: true, outcome: "PASS", selection }, null, 2));
  console.log(`PASS GH #551 / #387: ${out}`);
} catch (error) {
  try { screenshot("failure.png"); } catch {}
  const observed = await browser?.getPageSource().catch(() => "unavailable");
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ testedCommit, journey, expected: "Empty prompt after removal; selection toggles with actions above typed rows", observed: String(error), classification: "ambiguous", dom: observed }, null, 2));
  throw error;
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
