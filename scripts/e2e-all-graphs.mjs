import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { editorSelector } from "./lib/e2e-block-editor.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/og-shortcuts-758-757");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const alpha = path.join(run, "alpha"), beta = path.join(run, "beta");
for (const graph of [alpha, beta]) {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
  fs.writeFileSync(path.join(graph, "pages/Notes.md"), `- ${graph === alpha ? "Alpha" : "Beta"} notes\n`);
}
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({
  known_graphs: [{ name: "alpha", path: alpha }, { name: "beta", path: beta }], last_graph_path: alpha,
}));
const env = { ...process.env, TINE_GRAPH: alpha, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4892);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "all-graphs") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Notes");
  stage = "edit in alpha";
  await browser.$(".block-content").click();
  await browser.$(editorSelector()).waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  await browser.keys(" saved in alpha");
  await browser.keys(["Escape"]);
  stage = "open All graphs with hotstring";
  await browser.keys("g");
  await browser.keys(["Shift", "g"]);
  await browser.$(".all-graphs").waitForExist({ timeout: 10_000 });
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.hasAttribute("data-graph-open")), { timeout: 5000 });
  stage = "choose beta from the keyboard";
  // Find the target's ordinal from the remembered list; no ordering is mandated.
  const index = await browser.execute(target => [...document.querySelectorAll("[data-graph-open]")].findIndex(button => button.title === target), beta);
  assert.ok(index >= 0, "beta must be in the existing graph registry");
  await browser.keys(["Home"]);
  for (let i = 0; i < index; i++) await browser.keys(["ArrowDown"]);
  await browser.keys(["Enter"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".graph-switch-name")?.textContent?.trim() === "beta"), { timeout: 20_000 });
  await openPageByName(browser, "Notes");
  await browser.waitUntil(async () => (await browser.$("body").getText()).includes("Beta notes"), { timeout: 10_000 });
  stage = "verify graph ownership on disk";
  await waitForFileText(path.join(alpha, "pages/Notes.md"), text => text.includes("saved in alpha"));
  assert.equal(fs.readFileSync(path.join(beta, "pages/Notes.md"), "utf8"), "- Beta notes\n");
  // GH #1015: the switcher's rows carry no remove control; removing a graph is a row action on All graphs.
  const switcherRows = async () => {
    await browser.$(".graph-switch-btn").click();
    await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".graph-switch-row").length > 0), { timeout: 10_000 });
    const rows = await browser.execute(() => [...document.querySelectorAll(".graph-switch-row")].map(row => ({
      name: row.querySelector(".graph-switch-row-name")?.textContent?.trim(), controls: row.querySelectorAll("button, [role=button]").length,
    })));
    await browser.keys(["Escape"]);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector(".graph-switch-menu")), { timeout: 5000 });
    return rows;
  };
  stage = "switcher rows have no remove control";
  const before = await switcherRows();
  assert.deepEqual(before.map(row => row.name).sort(), ["alpha", "beta"]);
  assert.ok(before.every(row => row.controls === 0), `no switcher row may hold a control: ${JSON.stringify(before)}`);
  stage = "remove alpha from All graphs";
  await browser.keys("g");
  await browser.keys(["Shift", "g"]);
  await browser.$(".all-graphs").waitForExist({ timeout: 10_000 });
  const clicked = await browser.execute(() => {
    const row = [...document.querySelectorAll(".all-graphs-row")].find(item => item.getAttribute("aria-label") === "alpha");
    const remove = [...(row?.querySelectorAll("button") ?? [])].find(button => button.textContent === "Remove from this list");
    remove?.click();
    return Boolean(remove);
  });
  assert.ok(clicked, "All graphs must offer Remove from this list on alpha's row");
  await browser.waitUntil(() => browser.execute(() => ![...document.querySelectorAll(".all-graphs-row")].some(item => item.getAttribute("aria-label") === "alpha")), { timeout: 10_000 });
  const settings = path.join(run, "data", APP_ID, "tine-settings.json");
  await waitForFileText(settings, text => !(JSON.parse(text).known_graphs ?? []).some(graph => graph.path === alpha));
  stage = "alpha is gone from the switcher";
  const after = await switcherRows();
  assert.deepEqual(after.map(row => row.name), ["beta"]);
  assert.ok(fs.existsSync(path.join(alpha, "pages/Notes.md")), "removing from the list leaves the graph's files");
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "g Shift+G, keyboard graph switch, originating edit saved; switcher rows without remove control, remove from All graphs", alpha, beta }, null, 2));
  console.log(`PASS All graphs keyboard switch; alpha edit persisted, beta unchanged; no remove control in the switcher; alpha removed from All graphs and gone from the switcher. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nExpected: keyboard graph switch to beta; alpha edit saved only in alpha; switcher rows hold no control; alpha removed from All graphs leaves the switcher and settings\nObserved: ${error.stack}\nClassification: ambiguous pending driver log and screenshot\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
