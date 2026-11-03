// Native journey for CM step 2 batch 8: prose before a page's first bullet
// (Markdown) or first heading (Org) is that page's first block for every
// reference question, as OG's pre-block (graph-parser `block.cljs
// with-pre-block-if-exists`). All pages counts it and lists a page only it
// names; it is a Linked References row of its target; a rename through the
// page title rewrites it on disk while the page properties stay page
// properties. All pages also hides UUID names and `draws/` names (OG
// `get-all-pages`). Written by the cm-preamble lane.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm-preamble");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
// Each page: its text before the rename of Target to New, and after it.
const pages = {
  "pages/Target.md": ["- target body\n", null],
  "pages/Prose.md": ["[[Target]] in prose\n\n- a bullet\n", "[[New]] in prose\n\n- a bullet\n"],
  "pages/Mixed.md": ["tags:: Other\n\nsee [[Target]] and #Ghost\n\n- [[Target]] bullet\n", "tags:: Other\n\nsee [[New]] and #Ghost\n\n- [[New]] bullet\n"],
  "pages/Notes.org": ["Org prose [[Target]]\n\n* heading\n", "Org prose [[New]]\n\n* heading\n"],
  "pages/6571a2b3-0000-4000-8000-00000000c8b8.md": ["- named by a uuid\n", null],
  "pages/Sketch.md": ["title:: draws/sketch\n\n- a drawing page\n", null],
};
for (const [rel, [before]] of Object.entries(pages)) fs.writeFileSync(path.join(graph, rel), before);
fs.writeFileSync(path.join(graph, "journals", "2026_11_03.md"), "- journal\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4924);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";

/** All pages rows as { title: count text }. */
const allPagesRows = () => browser.execute(() => Object.fromEntries([...document.querySelectorAll(".all-pages-row")].map((row) =>
  [row.querySelector(".all-pages-open")?.textContent?.trim() ?? "", row.querySelector(".all-pages-backlinks")?.textContent?.trim() ?? ""])));
const linkedCount = () => browser.execute(() => {
  const header = [...document.querySelectorAll(".main-content .references-header, .references-header")]
    .find((el) => !el.closest(".unlinked-references") && el.textContent?.includes("Linked Reference"));
  return header?.querySelector(".references-count")?.textContent?.trim() ?? null;
});
const refRowTexts = () => browser.execute(() => [...document.querySelectorAll(".ref-block")]
  .filter((el) => !el.closest(".unlinked-references"))
  .map((el) => el.querySelector(".block-content")?.textContent?.trim() ?? ""));
async function openAllPages() {
  if (!await browser.$(".nav-all-pages-open").isExisting()) {
    await browser.execute(() => [...document.querySelectorAll(".nav-section-header")].find((header) => header.textContent?.includes("ALL PAGES"))?.click());
  }
  await browser.$(".nav-all-pages-open").waitForExist({ timeout: 15_000 });
  await browser.$(".nav-all-pages-open").click();
  await browser.$(".all-pages-row").waitForExist({ timeout: 15_000 });
}

try {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-preamble") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });

  stage = "All pages counts the preamble and lists a page only the preamble names";
  await openAllPages();
  // Target: Prose's preamble, Mixed's preamble and its bullet, Notes' Org prose.
  const want = { Target: "4", Ghost: "1", Other: "1" };
  let rows = {};
  try {
    await browser.waitUntil(async () => {
      rows = await allPagesRows();
      return Object.entries(want).every(([title, count]) => rows[title] === count);
    }, { timeout: 20_000, interval: 300 });
  } catch {
    assert.fail(`All pages rows ${JSON.stringify(rows)}, wanted at least ${JSON.stringify(want)}`);
  }
  stage = "All pages hides a UUID name and a draws/ name (OG get-all-pages)";
  assert.ok(!Object.keys(rows).some((title) => title.includes("00000000c8b8")), `no uuid row: ${JSON.stringify(rows)}`);
  assert.ok(!Object.keys(rows).some((title) => title.toLowerCase().startsWith("draws/")), `no draws/ row: ${JSON.stringify(rows)}`);

  stage = "Target's Linked References list the preamble rows";
  await openPageByName(browser, "Target");
  await browser.waitUntil(async () => (await linkedCount()) === "4", { timeout: 20_000, timeoutMsg: `Target header count ${await linkedCount()}` });
  let texts = [];
  await browser.waitUntil(async () => {
    texts = await refRowTexts();
    return texts.some((text) => text.includes("in prose")) && texts.some((text) => text.includes("Org prose")) && texts.some((text) => text.startsWith("see"));
  }, { timeout: 20_000, timeoutMsg: `Target's rows ${JSON.stringify(texts)}` });

  stage = "rename Target to New through the page title";
  await browser.execute(() => {
    document.querySelector("h1.page-title")?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.$(".page-title-input").waitForExist({ timeout: 4000, timeoutMsg: "the rename input never appeared" });
  await browser.execute(() => {
    const input = document.querySelector(".page-title-input");
    input.focus();
    input.value = "New";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "New", { timeout: 15_000, timeoutMsg: "the page title never became New" });
  await waitForFileText(path.join(graph, "pages/New.md"), (text) => text === "- target body\n", { timeoutMs: 15_000 });
  for (const [rel, [before, after]] of Object.entries(pages)) {
    stage = `${rel} after the rename`;
    if (after === null) {
      if (rel !== "pages/Target.md") assert.equal(fs.readFileSync(path.join(graph, rel), "utf8"), before, `${rel} unchanged`);
      continue;
    }
    await waitForFileText(path.join(graph, rel), (text) => text === after, { timeoutMs: 15_000 });
  }

  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "preamble prose counts in All pages (Target 4, Ghost 1), is a Linked References row, and is renamed (Markdown with and without page properties, Org); All pages hides a uuid name and a draws/ name", graph }, null, 2));
  console.log(`PASS CM preamble. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  const files = Object.keys(pages).concat("pages/New.md").map((rel) => {
    const file = path.join(graph, rel);
    return `${rel}: ${fs.existsSync(file) ? JSON.stringify(fs.readFileSync(file, "utf8")) : "(absent)"}`;
  }).join("\n");
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nExpected: the journey's rows and the pages' bytes after the rename\nObserved: ${error.stack}\n${files}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
