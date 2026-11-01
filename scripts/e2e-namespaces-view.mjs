// Native journey for the full Namespaces view (GH #335, D-20261101-L5).
// A graph of ~600 namespaced pages: the sidebar Namespaces section still expands inline;
// "Open the full view…" opens the view in the main pane; only a window of rows exists;
// Expand all / Collapse all and the size column work; a filter lists matches with their
// full path; clicking a name opens that page; the view survives a restart as the active
// tab. Written by the og-namespaces-335 lane; the manager runs native journeys.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/og-namespaces-335");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true :file/name-format :triple-lowbar}\n");
const AREAS = 6, TOPICS = 100;
const names = ["Project", "Project/Roadmap", "Project/Roadmap/2026", "Project/Notes", "Other/Notes", "Plain"];
for (let a = 0; a < AREAS; a++) for (let t = 0; t < TOPICS; t++) names.push(`Area ${a}/Topic ${String(t).padStart(3, "0")}`);
for (const name of names) fs.writeFileSync(path.join(graph, "pages", `${name.replaceAll("/", "___")}.md`), `- ${name} body\n`);
fs.writeFileSync(path.join(graph, "journals", "2026_11_01.md"), "- journal\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4896);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";

async function start() {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "namespaces-view") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
}
async function stop() {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  await new Promise((resolve) => setTimeout(resolve, 1500));
}
const labels = () => browser.execute(() => [...document.querySelectorAll(".ns-view-open")].map((el) => el.title));
const rowCount = () => browser.execute(() => document.querySelectorAll(".ns-view-row").length);
const click = (selector, text) => browser.execute((sel, label) => {
  const el = [...document.querySelectorAll(sel)].find((node) => label === undefined || node.textContent?.trim() === label);
  if (!(el instanceof HTMLElement)) return false;
  el.click();
  return true;
}, selector, text);

try {
  await start();
  stage = "expand the sidebar Namespaces section (inline tree is kept)";
  await browser.waitUntil(() => click(".nav-section-header", "NAMESPACES"), { timeout: 20_000, interval: 300 });
  await browser.$(".ns-tree").waitForExist({ timeout: 10_000 });
  assert.ok(await browser.$(".nav-namespaces-open").isExisting(), "the sidebar offers the full view");

  stage = "open the full view";
  assert.ok(await click(".nav-namespaces-open"));
  await browser.$(".ns-view").waitForExist({ timeout: 10_000 });
  assert.equal(await currentPageTitle(browser), "Namespaces");
  // Default: top-level namespaces open. Area 0..5, Other, Project are the 8 roots.
  await browser.waitUntil(async () => (await labels()).includes("Project/Roadmap"), { timeout: 10_000 });
  const initial = await rowCount();
  assert.ok(initial > 0 && initial < 80, `only a window of rows is drawn (${initial})`);
  assert.ok(await browser.execute(() => document.querySelector(".ns-tree") !== null), "the sidebar tree is still inline");

  stage = "collapse all, expand all, sizes";
  assert.ok(await click(".ns-view-action", "Collapse all"));
  await browser.waitUntil(async () => (await labels()).length === 8, { timeout: 5000 });
  const projectSize = await browser.execute(() => {
    const row = [...document.querySelectorAll(".ns-view-row")].find((r) => r.querySelector(".ns-view-open")?.title === "Project");
    return row?.querySelector(".ns-view-count")?.textContent;
  });
  assert.equal(projectSize, "3", "Project holds Roadmap, Roadmap/2026 and Notes");
  assert.ok(await click(".ns-view-action", "Expand all"));
  await browser.waitUntil(async () => (await labels()).includes("Project/Roadmap/2026") || (await rowCount()) > 20, { timeout: 5000 });
  assert.ok((await rowCount()) < 80, "expand all still draws a window");

  stage = "filter shows full paths";
  await browser.$(".all-pages-filter").setValue("notes");
  await browser.waitUntil(async () => (await labels()).join("|") === "Other/Notes|Project/Notes", { timeout: 5000 });
  await browser.$(".all-pages-filter").setValue("");
  await browser.waitUntil(async () => (await labels()).length > 2, { timeout: 5000 });

  stage = "open a page from the view";
  assert.ok(await browser.execute(() => {
    const el = [...document.querySelectorAll(".ns-view-open")].find((node) => node.title === "Project/Roadmap");
    if (!(el instanceof HTMLElement)) return false;
    el.click();
    return true;
  }));
  await browser.waitUntil(async () => (await currentPageTitle(browser)).endsWith("Roadmap"), { timeout: 10_000 });

  stage = "the view comes back after a restart";
  const back = await browser.$('button[title="Go back"]');
  assert.ok(await back.isExisting() && await back.isEnabled(), "Go back is available after opening a page from the view");
  await back.click();
  await browser.$(".ns-view").waitForExist({ timeout: 10_000 });
  await new Promise((resolve) => setTimeout(resolve, 2500)); // let the debounced session save land
  await stop();
  await start();
  await browser.$(".ns-view").waitForExist({ timeout: 20_000 });
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "sidebar entry, windowed tree, expand/collapse all, sizes, path filter, open page, restart", graph }, null, 2));
  console.log(`PASS Namespaces view. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nExpected: the Namespaces view behaves as the journey describes\nObserved: ${error.stack}\nClassification: ambiguous pending driver log and screenshot\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
