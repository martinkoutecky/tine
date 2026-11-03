// Native journey for CM step 2 batch 5 (D-20261102-CM4 CM-COUNT, CM-REFONLY).
// All pages shows each page's count as the number of blocks its unfiltered
// Linked References panel lists (self and code excluded, a block once however
// it names the page), lists a page that only blocks name (a reference-only
// page) with its own count, hides OG built-in pages (TODO; LATER even with a
// file, OG `get-all-pages`) through the Rust answer, names no page for a block
// embed, and opens a reference-only page whose Linked References header
// shows the same number. A page query (`(all-page-tags)`) lists a
// reference-only tag page by name. Written by the cm-counts-refonly lane.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm-counts-refonly");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
// Target's panel lists one block: Notes' first block (two links and a
// property value in one block count once); its own block and the code span
// do not count.
fs.writeFileSync(path.join(graph, "pages/Target.md"), "- [[Target]] names itself\n");
fs.writeFileSync(path.join(graph, "pages/Notes.md"), [
  "tags:: Ideas", "",
  "- see [[Target]] and [[Target]]", "  rel:: [[Target]]",
  "- `[[Target]]` in code",
  "- [[Ref Only]] first",
  "- #[[Ref Only]] second",
  "- TODO a task",
  "- {{embed ((00000000-0000-4000-8000-0000000000e1))}}",
  "- embedded", "  id:: 00000000-0000-4000-8000-0000000000e1", "",
].join("\n"));
// A file named like an OG built-in page: OG's All pages hides it too.
fs.writeFileSync(path.join(graph, "pages/LATER.md"), "- a later file\n");
fs.writeFileSync(path.join(graph, "pages/Queries.md"), '- {{query (all-page-tags) {:title "Tag pages"}}}\n');
fs.writeFileSync(path.join(graph, "journals", "2026_11_03.md"), "- journal\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4906);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";

async function start() {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-counts-refonly") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
}
/** All pages rows as { title: count text }. */
const allPagesRows = () => browser.execute(() => Object.fromEntries([...document.querySelectorAll(".all-pages-row")].map((row) =>
  [row.querySelector(".all-pages-open")?.textContent?.trim() ?? "", row.querySelector(".all-pages-backlinks")?.textContent?.trim() ?? ""])));
const linkedCount = () => browser.execute(() => {
  const header = [...document.querySelectorAll(".main-content .references-header, .references-header")]
    .find((el) => !el.closest(".unlinked-references") && el.textContent?.includes("Linked Reference"));
  return header?.querySelector(".references-count")?.textContent?.trim() ?? null;
});
async function openAllPages() {
  if (!await browser.$(".nav-all-pages-open").isExisting()) {
    await browser.execute(() => [...document.querySelectorAll(".nav-section-header")].find((header) => header.textContent?.includes("ALL PAGES"))?.click());
  }
  await browser.$(".nav-all-pages-open").waitForExist({ timeout: 15_000 });
  await browser.$(".nav-all-pages-open").click();
  await browser.$(".all-pages-row").waitForExist({ timeout: 15_000 });
}

try {
  await start();
  stage = "All pages counts are the Linked References panel sizes; a reference-only page is listed";
  await openAllPages();
  const want = { Target: "1", Notes: "0", "Ref Only": "2", Ideas: "1", rel: "1" };
  let rows = {};
  try {
    await browser.waitUntil(async () => {
      rows = await allPagesRows();
      return Object.entries(want).every(([title, count]) => rows[title] === count);
    }, { timeout: 20_000, interval: 300 });
  } catch {
    assert.fail(`All pages rows ${JSON.stringify(rows)}, wanted at least ${JSON.stringify(want)}`);
  }
  stage = "OG built-in pages are hidden (A11)";
  assert.ok(!Object.keys(rows).some((title) => title.toUpperCase() === "TODO"), `no TODO row: ${JSON.stringify(rows)}`);
  assert.ok(!Object.keys(rows).some((title) => title.toUpperCase() === "LATER"), `no LATER row though LATER.md exists: ${JSON.stringify(rows)}`);
  stage = "a block embed names no page (batch 5 ruling)";
  assert.ok(!Object.keys(rows).some((title) => title.includes("0000000000e1")), `no embed uuid row: ${JSON.stringify(rows)}`);

  stage = "a file page's Linked References header matches its All pages count";
  await openPageByName(browser, "Target");
  await browser.waitUntil(async () => (await linkedCount()) === "1", { timeout: 20_000, timeoutMsg: `Target header count ${await linkedCount()}` });

  stage = "a reference-only page opens from All pages and its header matches";
  await openAllPages();
  assert.ok(await browser.execute(() => {
    const el = [...document.querySelectorAll(".all-pages-open")].find((node) => node.textContent?.trim() === "Ref Only");
    if (!(el instanceof HTMLElement)) return false;
    el.click();
    return true;
  }), "the Ref Only row is clickable");
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Ref Only", { timeout: 10_000 });
  await browser.waitUntil(async () => (await linkedCount()) === "2", { timeout: 20_000, timeoutMsg: `Ref Only header count ${await linkedCount()}` });
  assert.ok(!fs.existsSync(path.join(graph, "pages/Ref Only.md")), "opening a reference-only page writes no file");

  stage = "a page query lists the reference-only tag page by name (A15)";
  await openPageByName(browser, "Queries");
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".query-page-link")].some((el) => el.textContent?.trim() === "Ideas")),
    { timeout: 20_000, timeoutMsg: "the Tag pages query never listed Ideas" });
  assert.ok(await browser.execute(() => {
    const el = [...document.querySelectorAll(".query-page-link")].find((node) => node.textContent?.trim() === "Ideas");
    if (!(el instanceof HTMLElement)) return false;
    el.click();
    return true;
  }));
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Ideas", { timeout: 10_000 });
  await browser.waitUntil(async () => (await linkedCount()) === "1", { timeout: 20_000, timeoutMsg: `Ideas header count ${await linkedCount()}` });

  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "All pages counts = panel sizes, reference-only rows, built-in hidden (also with a file), no embed uuid row, open reference-only page, (all-page-tags) lists a reference-only tag page", graph }, null, 2));
  console.log(`PASS CM counts and reference-only pages. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nExpected: the journey's counts and rows\nObserved: ${error.stack}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
