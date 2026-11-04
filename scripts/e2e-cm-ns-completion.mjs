// Native journey for CM step 2 batch 14 (S8 follow-ups, OG `namespace-page?`
// and `search/db.cljs make-pages-title-indice!`). Typing `[[` offers a
// namespace parent as a page (`Work` of `Work/Plan`, which only a reference
// names), once, and inserting it writes `[[Work]]`; a relative path or a URL
// (`./rel/x`, `https://a.b/c`) makes no parent, neither in the completion
// popup (`.`, `..`, `https:`) nor in All pages. Written by the cm-ns-followups lane.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm-ns-completion");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
fs.writeFileSync(path.join(graph, "pages/Notes.md"), "- [[Work/Plan]] [[./rel/x]] [[../up/y]] [[https://a.b/c]]\n");
fs.writeFileSync(path.join(graph, "pages/Scratch.md"), "- scratch\n");
fs.writeFileSync(path.join(graph, "journals", "2026_11_04.md"), "- journal\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4907);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";

async function start() {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-ns-completion") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
}
const labels = () => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")].map((el) => el.textContent.trim()));
/** Type `text` into the open block editor as input and return the choices once
 *  `ready` accepts them (the popup refreshes on a debounce). */
async function complete(text, ready) {
  await browser.execute((value) => {
    const editor = document.querySelector("textarea.block-editor");
    editor.focus(); editor.value = value; editor.setSelectionRange(value.length, value.length);
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value.at(-1) }));
  }, text);
  let current = [];
  try {
    await browser.waitUntil(async () => { current = (await labels()).filter((name) => !name.startsWith("Create")); return ready(current); },
      { timeout: 15_000, interval: 200 });
  } catch {
    assert.fail(`completion for ${JSON.stringify(text)} showed ${JSON.stringify(current)}`);
  }
  return current;
}
async function allPagesTitles() {
  if (!await browser.$(".nav-all-pages-open").isExisting()) {
    await browser.execute(() => [...document.querySelectorAll(".nav-section-header")].find((header) => header.textContent?.includes("ALL PAGES"))?.click());
  }
  await browser.$(".nav-all-pages-open").waitForExist({ timeout: 15_000 });
  await browser.$(".nav-all-pages-open").click();
  await browser.$(".all-pages-row").waitForExist({ timeout: 15_000 });
  return browser.execute(() => [...document.querySelectorAll(".all-pages-row .all-pages-open")].map((el) => el.textContent?.trim() ?? ""));
}

try {
  await start();
  stage = "typing [[Wor offers the namespace parent Work, once, beside Work/Plan";
  await openPageByName(browser, "Scratch");
  await browser.$(".ls-block .block-content-wrapper").waitForExist({ timeout: 20_000 });
  await browser.$(".ls-block .block-content-wrapper").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5_000 });
  const choices = await complete("[[Wor", (names) => names.includes("Work") && names.includes("Work/Plan"));
  assert.equal(choices.filter((name) => name === "Work").length, 1, `Work offered once: ${JSON.stringify(choices)}`);

  stage = "choosing the parent inserts [[Work]]";
  await browser.execute(() => {
    const row = [...document.querySelectorAll(".autocomplete .ac-label")].find((el) => el.textContent.trim() === "Work").parentElement;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
  });
  await browser.waitUntil(async () => (await browser.$("textarea.block-editor").getValue()).trim() === "[[Work]]", { timeout: 5_000 });

  stage = "a relative path or a URL is no namespace parent in the popup";
  const dots = await complete("[[.", (names) => names.includes("./rel/x") && names.includes("../up/y"));
  assert.ok(!dots.includes(".") && !dots.includes(".."), `no . or .. page offered: ${JSON.stringify(dots)}`);
  const urls = await complete("[[https", (names) => names.includes("https://a.b/c"));
  assert.ok(!urls.includes("https:") && !urls.includes("https://a.b"), `no URL prefix offered: ${JSON.stringify(urls)}`);

  stage = "All pages lists the parent and no relative or URL prefix";
  await browser.keys("Escape");
  const titles = await allPagesTitles();
  assert.ok(titles.includes("Work"), `All pages lists Work: ${JSON.stringify(titles)}`);
  for (const bad of [".", "..", "../up", "https:", "https://a.b"]) {
    assert.ok(!titles.includes(bad), `All pages has no ${bad}: ${JSON.stringify(titles)}`);
  }
  assert.ok(!fs.existsSync(path.join(graph, "pages/Work.md")), "offering a parent writes no file");

  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "[[ completion offers a namespace parent once and inserts it; ./x, ../x and URL names make no parent in the popup or All pages", graph }, null, 2));
  console.log(`PASS CM namespace parents in completion. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nExpected: the journey's choices and rows\nObserved: ${error.stack}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
