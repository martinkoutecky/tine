// Linux real-WebKit journey for the og query block (batch 14q4a, ported in
// meaning from master's scripts/e2e-query-sheet.mjs): a query block rests as a
// sentence and answers with blocks or pages, a query the engine cannot read
// shows its diagnostics instead of "no results" (I-9), an empty answer explains
// itself, and a query created with /query and saved from the sheet's text pane
// lands on disk as an ordinary query block that reopens after a restart.
//
// A real engine is the point: every reading and every print here goes through
// Rust (parseQuery / printQuery / queryRun / queryExplainEmpty), which jsdom
// tests can only mock.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4496);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4497);
const TMP_ROOT = path.resolve(process.env.E2E_TMP_ROOT || process.env.TMPDIR || "/tmp");
fs.mkdirSync(TMP_ROOT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(TMP_ROOT, "tine-og-query-sheet-e2e-"));
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Queries]]\n");
fs.writeFileSync(`${GRAPH}/pages/Tasks.md`, "- TODO alpha task\n- TODO beta task\n- DONE finished task\n");
fs.writeFileSync(`${GRAPH}/pages/Book A.md`, "type:: book\n\n- A book page\n");
fs.writeFileSync(`${GRAPH}/pages/Notes.md`, "type:: note\n\n- Not a book\n");
const QUERIES_FILE = `${GRAPH}/pages/Queries.md`;
// Order matters: the journey finds each query block by its position.
const INITIAL = [
  "- {{query (task TODO)}}",
  "- {{query (page-property type book)}}",
  "- {{tine-query @block and nosuchfield('x')}}",
  "- {{query (and (task TODO) (priority C))}}",
  "",
].join("\n");
fs.writeFileSync(QUERIES_FILE, INITIAL);

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

async function withApp(index, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const log = fs.openSync(`${TMP}/tauri-driver-${index}.log`, "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
    env, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
      connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
    try {
      await fn(browser);
    } catch (error) {
      const state = await browser.execute(() => ({ text: document.body.innerText })).catch(() => null);
      fs.writeFileSync(`${ARTIFACTS}/failure-state-${index}.json`, `${JSON.stringify(state, null, 2)}\n`);
      try { await browser.saveScreenshot(`${ARTIFACTS}/failure-${index}.png`); } catch {}
      throw error;
    }
    // Let the debounced save and session write settle before the orderly exit.
    await sleep(1_500);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

/** The text of the Nth query block on the routed page (document order). */
const queryText = (browser, index) => browser.execute((i) => {
  const block = document.querySelectorAll(".page-blocks .query-block")[i];
  return block ? (block.textContent ?? "").replace(/\s+/g, " ") : null;
}, index);

async function waitForQuery(browser, index, predicate, what) {
  // Query results hydrate when approached. Observing an offscreen group's
  // reserved-height shell does not prove whether its answer rendered.
  await browser.waitUntil(() => browser.execute((i) => {
    const block = document.querySelectorAll(".page-blocks .query-block")[i];
    if (!block) return false;
    block.scrollIntoView({ block: "center" });
    return true;
  }, index), { timeout: 10_000, interval: 100, timeoutMsg: `query block ${index} did not mount` });
  let last = null;
  await browser.waitUntil(async () => {
    last = await queryText(browser, index);
    return last !== null && predicate(last);
  }, { timeout: 20_000, interval: 150, timeoutMsg: "timed out" }).catch(() => {
    throw new Error(`query block ${index}: ${what}; its text was ${JSON.stringify(last)}`);
  });
  return last;
}

const disk = () => fs.readFileSync(QUERIES_FILE, "utf8");

let createdLine = null;

await withApp(0, async (browser) => {
  await openPageByName(browser, "Queries");

  // 1. A block query rests as a sentence and answers with blocks.
  await waitForQuery(browser, 0, (t) => t.includes("alpha task") && t.includes("beta task"), "block answers never landed");
  const first = await queryText(browser, 0);
  if (first.includes("finished task")) throw new Error("a DONE block answered a (task TODO) query");
  if (!(await browser.execute(() => !!document.querySelectorAll(".page-blocks .query-block")[0]?.querySelector(".qs-sentence")))) {
    throw new Error("the block query did not rest as a sentence");
  }

  // 2. A page-level filter answers with pages.
  await waitForQuery(browser, 1, (t) => t.includes("Book A"), "page answers never landed");
  const pages = await browser.execute(() => [...document.querySelectorAll(".page-blocks .query-block")[1]
    .querySelectorAll(".query-page-link")].map((link) => (link.textContent ?? "").trim()));
  if (!pages.includes("Book A") || pages.includes("Notes")) throw new Error(`page answers were ${JSON.stringify(pages)}`);

  // 3. A query the engine cannot read shows its diagnostics, never a bare "No results" (I-9).
  await waitForQuery(browser, 2, (t) => t.includes("didn't understand part of this query"), "the unreadable query showed no diagnostics");

  // 4. An empty answer explains itself.
  await waitForQuery(browser, 3, (t) => /why empty\?/.test(t), "the empty query offered no why-empty");
  await browser.execute(() => {
    const button = document.querySelectorAll(".page-blocks .query-block")[3]?.querySelector(".query-why-empty");
    if (button instanceof HTMLElement) button.click();
  });
  await browser.waitUntil(() => browser.execute(() => {
    const table = document.querySelectorAll(".page-blocks .query-block")[3]?.querySelector(".query-why-empty-table");
    return !!table && table.querySelectorAll("tbody tr").length >= 2;
  }), { timeout: 15_000, interval: 150, timeoutMsg: "why-empty never listed the query's conditions" });

  if (disk() !== INITIAL) throw new Error(`reading queries wrote the page:\n${disk()}`);

  // 5. Create a query: /query opens the sheet; the text pane saves it.
  await browser.execute(() => {
    const target = document.querySelector(".page-trailing-block-target");
    if (target instanceof HTMLElement) target.click();
  });
  const editor = await browser.$(".page-blocks textarea");
  await editor.waitForExist({ timeout: 10_000 });
  // The sheet anchors below this sentence. Leave room for its controls after
  // the preceding probes have scrolled through the query answers.
  await editor.scrollIntoView({ block: "start", inline: "nearest" });
  await editor.addValue("/query");
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".autocomplete-item, .ac-item, [role='option']")]
      .some((item) => (item.textContent ?? "").trim().startsWith("Query"))), {
    timeout: 10_000, interval: 100, timeoutMsg: "the /query command was not offered",
  });
  await browser.keys(["Enter"]);
  const pane = await browser.$(".qs-sheet .query-text-pane-input");
  await pane.waitForExist({ timeout: 15_000 });
  // The field chooser opens first; Escape peels only that layer. Wait for the
  // chooser to be open before Escape and closed after it, or the keypress can
  // race the auto-open and leave the chooser over the pane.
  const chooserOpen = () => browser.execute(() => document.querySelector(".qs-sheet .qs-add")?.getAttribute("aria-expanded") === "true");
  await browser.waitUntil(chooserOpen, { timeout: 10_000, interval: 100, timeoutMsg: "/query did not open the field chooser" });
  await browser.keys(["Escape"]);
  await browser.waitUntil(async () => !(await chooserOpen()), { timeout: 10_000, interval: 100, timeoutMsg: "Escape did not close the field chooser" });
  await pane.waitForClickable({ timeout: 10_000 });
  await browser.execute(() => {
    const input = document.querySelector(".qs-sheet .query-text-pane-input");
    if (input instanceof HTMLElement) input.focus();
  });
  await pane.setValue("@block and content like '%alpha%'");
  const save = await browser.$(".qs-sheet .query-text-pane-save");
  await browser.waitUntil(async () => save.isEnabled(), { timeout: 15_000, interval: 150, timeoutMsg: "Save query text never enabled" });
  await save.click();
  await browser.waitUntil(() => /\{\{(tine-)?query [^\n]*alpha[^\n]*\}\}/i.test(disk()), {
    timeout: 15_000, interval: 150, timeoutMsg: "the saved query never reached the file",
  });
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".qs-sheet")), {
    timeout: 10_000, interval: 100, timeoutMsg: "Escape did not close the sheet",
  });
  const created = await waitForQuery(browser, 4, (t) => t.includes("alpha task"), "the created query never answered");
  if (created.includes("beta task")) throw new Error(`the created query did not filter: ${created}`);
  const lines = disk().split("\n");
  createdLine = lines.find((line) => /alpha/.test(line) && /\{\{(tine-)?query /i.test(line)) ?? null;
  if (lines.slice(0, 4).join("\n") !== INITIAL.split("\n").slice(0, 4).join("\n")) {
    throw new Error(`creating a query changed the other query blocks:\n${disk()}`);
  }
});

// 6. After a restart the created query reopens as the same query.
await withApp(1, async (browser) => {
  await openPageByName(browser, "Queries");
  const created = await waitForQuery(browser, 4, (t) => t.includes("alpha task"), "the created query did not answer after a restart");
  if (created.includes("beta task")) throw new Error(`the created query lost its filter after a restart: ${created}`);
  if (!disk().includes(createdLine)) throw new Error(`the created query's bytes changed across a restart:\n${disk()}`);
});

console.log(`PASS query sheet journey (${createdLine})`);
fs.rmSync(TMP, { recursive: true, force: true });
