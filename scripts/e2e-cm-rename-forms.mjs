// Native journey for CM step 2 batch 6: a rename rewrites what the reference
// answer reads as a reference, and nothing else. Renaming Old -> New through
// the page title rewrites a [[link]] in a cloze and in a query on pages whose
// only mention is that macro, a property key (OG's lower-cased spelling),
// `word#Old`, an Org labelled link and an Org headline tag; it leaves a code
// block whose fence opens on the bullet line and a URL's bracketed query
// string as written. Written by the cm-rename-scanner lane.
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
const artifacts = path.join(root, "artifacts/cm-rename-forms");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
// Each page: its text before the rename, and after it.
const pages = {
  "pages/Old.md": ["- old body\n", null],
  "pages/Cloze.md": ["- {{cloze [[Old]]}}\n", "- {{cloze [[New]]}}\n"],
  "pages/Query.md": ["- {{query [[Old]]}}\n", "- {{query [[New]]}}\n"],
  "pages/Key.md": ["- body\n  Old:: value\n", "- body\n  new:: value\n"],
  "pages/Word.md": ["- word#Old here\n", "- word#New here\n"],
  "pages/Labels.org": ["* see [[Old][a label]]\n* heading :Old:\n", "* see [[New][a label]]\n* heading :New:\n"],
  "pages/Fence.md": ["- ```js\n  [[Old]]\n  ```\n- [[Old]] after\n", "- ```js\n  [[Old]]\n  ```\n- [[New]] after\n"],
  "pages/Url.md": ["- see https://ex.com/q?[[[Old],[2]]] and [[Old]]\n", "- see https://ex.com/q?[[[Old],[2]]] and [[New]]\n"],
};
for (const [rel, [before]] of Object.entries(pages)) fs.writeFileSync(path.join(graph, rel), before);
fs.writeFileSync(path.join(graph, "journals", "2026_11_03.md"), "- journal\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4908);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";

try {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-rename-forms") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });

  stage = "rename Old to New through the page title";
  await openPageByName(browser, "Old");
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
  await waitForFileText(path.join(graph, "pages/New.md"), (text) => text === "- old body\n", { timeoutMs: 15_000 });
  assert.ok(!fs.existsSync(path.join(graph, "pages/Old.md")), "Old.md moved");

  for (const [rel, [, after]] of Object.entries(pages)) {
    if (after === null) continue;
    stage = `${rel} after the rename`;
    await waitForFileText(path.join(graph, rel), (text) => text === after, { timeoutMs: 15_000 });
  }

  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "rename Old -> New: cloze and query macro links on pages with no other mention, property key, word#tag, Org labelled link and headline tag follow; a fence on the bullet line and a URL's brackets stay", graph }, null, 2));
  console.log(`PASS CM rename forms. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  const files = Object.keys(pages).concat("pages/New.md").map((rel) => {
    const file = path.join(graph, rel);
    return `${rel}: ${fs.existsSync(file) ? JSON.stringify(fs.readFileSync(file, "utf8")) : "(absent)"}`;
  }).join("\n");
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nExpected: the pages' bytes after the rename\nObserved: ${error.stack}\n${files}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
