// Native journey for D-20261119-YL: a journal title format that names no
// single day is not used. The graph's config.edn sets
// `:journal/page-title-format "MM-dd"` (no year: `11-02` would be November 2nd
// of every year) and holds the journals 2026_11_02 and 2027_11_02.
//  1. The notice appears once for the graph load (a warning toast that
//     auto-dismisses) and is not repeated during the session.
//  2. Settings > Journals shows the format in use and why.
//  3. Both days carry the default title (`MMM do, yyyy`) and open by it.
//  4. Typing [[11-02]] in a block links an ordinary page named 11-02, not a
//     day.
//  5. After a reload the notice appears once again, and config.edn and every
//     journal file are byte-identical to what the journey wrote.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openJournals, openPageByLink, openPageByName } from "./lib/e2e-navigation.mjs";
import { editorSelector, editorText, pasteEditorText } from "./lib/e2e-block-editor.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

// The notice's meaning, not its English: a warning naming the format in use
// and the refused configured one (both quoted message arguments, the same in
// every locale). The wording is the catalog's and its unit tests'.
const names = (text) => text.includes('"MMM do, yyyy"') && text.includes('"MM-dd"');
const CONFIG = '{:journal/page-title-format "MM-dd"\n :tine/guide-announced? true}\n';

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm/cm-dated-titles");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
const seeded = {
  "logseq/config.edn": CONFIG,
  "journals/2026_11_02.md": "- day of 2026\n",
  "journals/2027_11_02.md": "- day of 2027\n",
  "pages/Scratch.md": "- scratch\n",
};
for (const [rel, text] of Object.entries(seeded)) fs.writeFileSync(path.join(graph, rel), text);
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const ownerFixture = accountHomeFixture(run, run);
const env = { ...ownerFixture.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4937);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
let browser;
let stage = "startup";

/** config.edn and every journal file, graph-relative path → bytes. */
function untouched() {
  const files = { "logseq/config.edn": fs.readFileSync(path.join(graph, "logseq/config.edn"), "utf8") };
  for (const name of fs.readdirSync(path.join(graph, "journals")).sort()) files[`journals/${name}`] = fs.readFileSync(path.join(graph, "journals", name), "utf8");
  return files;
}
const expectedUntouched = Object.fromEntries(Object.entries(seeded).filter(([rel]) => rel !== "pages/Scratch.md"));

/** The shown title-format notices: text (Fluent isolation marks removed) and
 *  whether the toast counts a repeat. */
const notices = () => browser.execute(() => [...document.querySelectorAll(".toast")]
  .map((toast) => ({
    warn: toast.classList.contains("toast-warn"),
    text: (toast.querySelector(".toast-msg")?.textContent ?? "").replace(/[⁨⁩]/g, "").trim(),
    repeated: toast.querySelector(".toast-count") !== null,
  }))
  .filter((toast) => toast.text.includes('"MM-dd"')));

/** Record every title-format notice this page shows from now on, one entry
 *  per toast element, so a notice that auto-dismisses is still counted.
 *  Installed as soon as the window exists, before the graph load publishes
 *  its config (a transient toast lives 3.2 s, src/toasts.ts). */
const recordNotices = () => browser.execute(() => {
  if (window.__titleNotices) return;
  const seen = (window.__titleNotices = []);
  const marked = new WeakSet();
  const scan = () => {
    for (const toast of document.querySelectorAll(".toast")) {
      const text = (toast.querySelector(".toast-msg")?.textContent ?? "").replace(/[⁨⁩]/g, "").trim();
      if (marked.has(toast) || !text.includes('"MM-dd"')) continue;
      marked.add(toast);
      seen.push({ warn: toast.classList.contains("toast-warn"), text, repeated: toast.querySelector(".toast-count") !== null });
    }
  };
  scan();
  new MutationObserver(scan).observe(document.documentElement, { childList: true, subtree: true, characterData: true });
});
const recorded = () => browser.execute(() => window.__titleNotices ?? []);
/** Wait for the graph load's notice, assert it was shown exactly once, as a
 *  warning naming both formats, and that it dismisses itself. */
async function noticeShownOnce(what) {
  await browser.waitUntil(async () => (await recorded()).length > 0, { timeout: 20_000, interval: 150, timeoutMsg: `${what}: the title-format notice never appeared` });
  const shown = await recorded();
  assert.equal(shown.length, 1, `${what}: the notice is shown once per load: ${JSON.stringify(shown)}`);
  assert.ok(shown[0].warn && !shown[0].repeated, `${what}: the notice is one warning: ${JSON.stringify(shown)}`);
  assert.ok(names(shown[0].text), `${what}: the notice names the format in use and the refused one: ${shown[0].text}`);
  await browser.waitUntil(async () => (await notices()).length === 0, { timeout: 10_000, interval: 150, timeoutMsg: `${what}: the notice did not dismiss itself` });
}

try {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-dated-titles") });
  await ensureMainWindow(browser);
  await recordNotices();
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await assertAppLease(ownerFixture);

  stage = "the graph load shows the title-format notice once";
  await noticeShownOnce("first load");

  stage = "Settings > Journals shows the format in use and why";
  await browser.execute(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await browser.keys(["t", "s"]);
  await browser.$('[data-settings-tab="journals"]').waitForExist({ timeout: 10_000, timeoutMsg: "Settings did not open" });
  await browser.$('[data-settings-tab="journals"]').click();
  const diagnostic = () => browser.execute(() => [...document.querySelectorAll('.settings-modal .config-diagnostic[role="status"]')]
    .map((node) => (node.textContent ?? "").replace(/[⁨⁩]/g, "").trim()));
  await browser.waitUntil(async () => (await diagnostic()).length > 0, { timeout: 10_000, interval: 100, timeoutMsg: "Settings > Journals shows no title-format line" });
  const lines = await diagnostic();
  assert.equal(lines.length, 1, `Settings > Journals shows one title-format line: ${JSON.stringify(lines)}`);
  assert.ok(names(lines[0]), `Settings > Journals names the format in use and the refused one: ${lines[0]}`);
  await browser.keys("Escape");
  await browser.$(".settings-modal").waitForExist({ reverse: true, timeout: 5_000, timeoutMsg: "Settings stayed open after Escape" });

  stage = "both days carry the default title";
  await openJournals(browser);
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".page-section[data-page-name]")]
    .some((section) => section.getAttribute("data-page-name") === "Nov 2nd, 2026")), { timeout: 15_000, interval: 150, timeoutMsg: "the journal feed shows no Nov 2nd, 2026" });
  const feed = await browser.execute(() => [...document.querySelectorAll(".page-section[data-page-name]")].map((section) => section.getAttribute("data-page-name")));
  assert.ok(!feed.includes("11-02"), `no journal is titled 11-02: ${JSON.stringify(feed)}`);
  for (const [title, body] of [["Nov 2nd, 2026", "day of 2026"], ["Nov 2nd, 2027", "day of 2027"]]) {
    await openPageByName(browser, title);
    await browser.waitUntil(() => browser.execute((body) => [...document.querySelectorAll(".main-content .ls-block .block-content")]
      .some((node) => (node.textContent ?? "").includes(body)), body), { timeout: 15_000, interval: 150, timeoutMsg: `${title} does not show its file's block "${body}"` });
  }

  stage = "typing [[11-02]] in a block links an ordinary page";
  await openPageByName(browser, "Scratch");
  await browser.$(".main-content .ls-block .block-content-wrapper").waitForExist({ timeout: 20_000 });
  await browser.$(".main-content .ls-block .block-content-wrapper").click();
  await browser.$(editorSelector()).waitForExist({ timeout: 5_000, timeoutMsg: "the block editor did not open" });
  // Text enters through the shared editor boundary's native clipboard path
  // (scripts/lib/e2e-block-editor.mjs), as the other CM journeys enter links:
  // a WebDriver key burst races the editor's bracket pairing.
  await pasteEditorText(browser, "scratch [[11-02]]");
  let typed = "";
  await browser.waitUntil(async () => (typed = await editorText(browser)) === "scratch [[11-02]]", { timeout: 5_000, interval: 100 })
    .catch(() => { throw new Error(`the typed link never reached the editor, which holds ${JSON.stringify(typed)}`); });
  // Escape closes the [[ popup if it opened, then leaves editing.
  await browser.waitUntil(async () => {
    const editing = await browser.execute((selector) => document.querySelector(selector) !== null, editorSelector());
    if (editing) await browser.keys("Escape");
    return !editing;
  }, { timeout: 5_000, interval: 250, timeoutMsg: "the block stayed in editing after Escape" });
  await waitForFileText(path.join(graph, "pages/Scratch.md"), (text) => text === "- scratch [[11-02]]\n", { timeoutMs: 15_000 });
  await openPageByLink(browser, "11-02");
  assert.equal(await currentPageTitle(browser), "11-02", "[[11-02]] opens the page 11-02, not a day");
  assert.equal(await browser.execute(() => document.querySelector(".page-title-row.journal-today") !== null), false, "11-02 is not shown as a journal day");
  assert.ok(!fs.existsSync(path.join(graph, "pages/11-02.md")), "opening the ordinary page writes no file");
  assert.deepEqual(untouched(), expectedUntouched, "config.edn and the journals are unchanged before the reload");

  stage = "the notice was not repeated during the session";
  assert.equal((await recorded()).length, 1, "the notice did not repeat during the session");
  assert.deepEqual(await notices(), [], "no notice is shown again");

  stage = "after a reload the notice appears once again and the files are unchanged";
  await browser.refresh();
  await recordNotices();
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await noticeShownOnce("after reload");
  await openPageByName(browser, "Nov 2nd, 2027");
  assert.deepEqual(untouched(), expectedUntouched, "config.edn and the journals are byte-identical after the reload");
  assert.equal(fs.readFileSync(path.join(graph, "pages/Scratch.md"), "utf8"), "- scratch [[11-02]]\n");

  ownerFixture.verify();
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", journey: "MM-dd title format refused: the notice shows once per load and dismisses itself; Settings > Journals names the format in use and why; Nov 2nd, 2026 and Nov 2nd, 2027 carry default titles and open by them; typed [[11-02]] opens the ordinary page 11-02 and writes no file for it; config.edn and both journals byte-identical after a reload, which shows the notice once again", graph }, null, 2));
  console.log(`PASS CM dated titles. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  let files = "";
  try { files = Object.entries(untouched()).map(([rel, text]) => `${rel}: ${JSON.stringify(text)}`).join("\n"); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nObserved: ${error.stack}\n${files}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  ownerFixture.close?.();
}
