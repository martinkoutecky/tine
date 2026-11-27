// GH #1052: a nested page reference opens the page each part names.
// Completion for the page `Lorem [[Impsum]]` inserts `[[Lorem [[Impsum]]]]`
// (as Logseq does); a click on the outer label opens the seeded page
// `pages/Lorem [[Impsum]].md`, an edit there saves into that file and no file
// named with the outer brackets appears; a click on the inner part opens `Impsum`.
// Synthetic scratch graph and app data only. Needs a custom-protocol build at
// TINE_APP or target/debug/tine.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { blurEditor, editorSelector, editorText, pasteEditorText } from "./lib/e2e-block-editor.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openJournals } from "./lib/e2e-navigation.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

ensurePrivateSessionBus();
await import("./lib/e2e-auto-account-home.mjs");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const binarySha256 = createHash("sha256").update(fs.readFileSync(APP)).digest("hex");
const TMP = path.join(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/gh1052"), String(Date.now()));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state", "home"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const OUTER_FILE = path.join(GRAPH, "pages/Lorem [[Impsum]].md");
fs.writeFileSync(OUTER_FILE, "- Seeded outer body\n");
fs.writeFileSync(path.join(GRAPH, "pages/Impsum.md"), "- Seeded inner body\n");
const seeded = fs.readdirSync(path.join(GRAPH, "pages")).sort();
const now = new Date();
const journal = path.join(GRAPH, "journals",
  `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}.md`);
fs.writeFileSync(journal, "- Link here\n");
const env = { ...process.env, TINE_GRAPH: GRAPH, HOME: path.join(TMP, "xdg/home"),
  XDG_DATA_HOME: path.join(TMP, "xdg/data"), XDG_CONFIG_HOME: path.join(TMP, "xdg/config"), XDG_CACHE_HOME: path.join(TMP, "xdg/cache"),
  XDG_STATE_HOME: path.join(TMP, "xdg/state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4752);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
const observations = [];
const outerLink = ".ls-block a.page-ref"; // the one link the nested reference renders
const innerLink = ".ls-block a.page-ref [data-nested-range]"; // its inner part
/** A native click on the outer label's own text: its left edge, away from the inner link. */
const clickOuterLabel = async () => {
  const outer = await browser.$(outerLink);
  await outer.waitForExist({ timeout: 10_000 });
  const { width } = await outer.getSize();
  await outer.click({ x: -Math.floor(width / 2) + 4 });
};
const waitForPage = async (title, body) => {
  try {
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === title, { timeout: 15_000 });
  } catch {
    throw new Error(`page ${JSON.stringify(title)} did not open; the page shown is ${JSON.stringify(await currentPageTitle(browser))}`);
  }
  await browser.waitUntil(() => browser.execute((body) => document.querySelector(".page")?.textContent?.includes(body) ?? false, body),
    { timeout: 15_000, timeoutMsg: `page ${title} did not show ${body}` });
};
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.waitUntil(async () => {
    await ensureMainWindow(browser);
    return browser.$(".app-container").isExisting();
  }, { timeout: 20_000, interval: 100, timeoutMsg: "the main app did not mount" });
  await openJournals(browser);
  const block = await browser.$(".ls-block .block-content-wrapper");
  await block.waitForExist({ timeout: 20_000 });
  await block.click();
  await browser.$(editorSelector()).waitForExist({ timeout: 5000 });
  await pasteEditorText(browser, "[[Lorem");
  const pick = () => browser.execute((name) => {
    const row = [...document.querySelectorAll(".autocomplete .ac-label")].find((el) => el.textContent.trim() === name)?.parentElement;
    row?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    return Boolean(row);
  }, "Lorem [[Impsum]]");
  await browser.waitUntil(pick, { timeout: 10_000, timeoutMsg: "completion did not offer Lorem [[Impsum]]" });
  await browser.waitUntil(async () => (await editorText(browser)).trim() === "[[Lorem [[Impsum]]]]", { timeout: 5000 });
  await blurEditor(browser);
  await waitForFileText(journal, (text) => text.includes("[[Lorem [[Impsum]]]]"));
  observations.push({ step: "completion saved", journal: fs.readFileSync(journal, "utf8") });

  await clickOuterLabel();
  await waitForPage("Lorem [[Impsum]]", "Seeded outer body");
  observations.push({ step: "outer click", title: await currentPageTitle(browser) });
  const body = await browser.$(".ls-block .block-content-wrapper");
  await body.click();
  await browser.$(editorSelector()).waitForExist({ timeout: 5000 });
  await pasteEditorText(browser, "Seeded outer body edited");
  await blurEditor(browser);
  await waitForFileText(OUTER_FILE, (text) => text.includes("Seeded outer body edited"));
  assert.deepEqual(fs.readdirSync(path.join(GRAPH, "pages")).sort(), seeded, "no page file other than the seeded ones");
  observations.push({ step: "edit saved", file: fs.readFileSync(OUTER_FILE, "utf8"), pages: seeded });

  await openJournals(browser);
  const inner = await browser.$(innerLink);
  await inner.waitForExist({ timeout: 10_000 });
  await inner.click();
  await waitForPage("Impsum", "Seeded inner body");
  observations.push({ step: "inner click", title: await currentPageTitle(browser) });
  assert.deepEqual(fs.readdirSync(path.join(GRAPH, "pages")).sort(), seeded);

  await browser.saveScreenshot(path.join(TMP, "nested-ref.png"));
  fs.writeFileSync(path.join(TMP, "receipt.json"), JSON.stringify({ testedCommit, binarySha256, outcome: "pass", observations }, null, 2));
  console.log(`PASS: nested reference parts open their own pages; ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
  fs.writeFileSync(path.join(TMP, "receipt.json"), JSON.stringify({ testedCommit, binarySha256, outcome: "fail", journey: "nested reference target",
    expected: "outer label opens the seeded Lorem [[Impsum]] page, inner opens Impsum, no bracketed file", observed: String(error),
    observations, pages: fs.readdirSync(path.join(GRAPH, "pages")), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
