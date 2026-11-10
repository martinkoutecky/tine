// CM step 2 batch 2 (OW-2, UI-CM-DUPLICATE-DAY-KEEPER): one journal day with
// two files, `2026_10_26.md` (the stem the default format writes) and
// `2026-10-26.md` (an ISO stem, also a date name). The day opens the least
// graph-relative path, `2026-10-26.md` (conceptual model §2.2).
// Settings > Backups & recovery > Duplicate journal days must
// mark that file alone as canonical and offer Merge for the other; before the
// fix both were marked canonical and neither offered Merge. A literal native
// click on Merge then folds the other file into the opened one, and opening the
// day shows both files' blocks.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { saveJourneyScreenshot } from "./lib/e2e-screenshot.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-cm-duplicate-day-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4744);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{:tine/guide-announced? true}\n");
const OPENED = "journals/2026-10-26.md";
const OTHER = "journals/2026_10_26.md";
const TITLE = "Oct 26th, 2026";
fs.writeFileSync(path.join(GRAPH, OPENED), "- shared line\n- only in iso\n");
fs.writeFileSync(path.join(GRAPH, OTHER), "- shared line\n- only in underscore\n");
fs.writeFileSync(path.join(GRAPH, "pages/Notes.md"), "- a page\n");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACT, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, graph: GRAPH, steps: [] };

/** The duplicate-day rows Settings shows: path, canonical badge, Merge offered. */
const rows = () => browser.execute(() => [...document.querySelectorAll(".settings-modal .journal-conflict-row")].map((row) => ({
  path: row.getAttribute("data-journal-conflict"),
  canonical: !!row.querySelector(".journal-conflict-keep"),
  merge: [...row.querySelectorAll("button")].some((button) => (button.textContent ?? "").trim() === "Merge"),
})));

/** Whether the main view's page shows `text`. */
const dayShows = (text) => browser.execute((wanted) => (document.querySelector(".main-content .page-blocks")?.textContent ?? "").includes(wanted), text);

/** Native click on the Merge button of the row for `file`. */
async function clickMerge(file) {
  const point = await browser.execute((wanted) => {
    const row = document.querySelector(`.settings-modal .journal-conflict-row[data-journal-conflict="${wanted}"]`);
    const button = row && [...row.querySelectorAll("button")].find((node) => (node.textContent ?? "").trim() === "Merge");
    if (!button) return null;
    button.scrollIntoView({ block: "center" });
    const rect = button.getBoundingClientRect();
    const x = Math.round(rect.x + rect.width / 2), y = Math.round(rect.y + rect.height / 2);
    return button.contains(document.elementFromPoint(x, y)) ? { x, y } : null;
  }, file);
  if (!point) throw new Error(`no clickable Merge button on the row for ${file}`);
  await browser.performActions([
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
  ]);
  await browser.releaseActions();
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });

  // The day is on screen (viewed, not edited) while Settings merges into it.
  await openPageByName(browser, TITLE);
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === TITLE && await dayShows("only in iso"), {
    timeout: 15_000, interval: 150, timeoutMsg: `the day ${TITLE} did not open on ${OPENED}`,
  });
  proof.steps.push(`${TITLE} is open on screen, showing ${OPENED}'s blocks`);

  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
  await browser.$("//button[contains(concat(' ', normalize-space(@class), ' '), ' settings-nav-item ') and normalize-space(.)='Backups & recovery']").click();
  await browser.waitUntil(async () => (await rows()).length === 2, {
    timeout: 15_000, interval: 200, timeoutMsg: "Backups & recovery did not list the two files of the duplicate day",
  });
  const listed = await rows();
  proof.listed = listed;
  const canonical = listed.filter((row) => row.canonical).map((row) => row.path);
  const mergeable = listed.filter((row) => row.merge).map((row) => row.path);
  if (JSON.stringify(canonical) !== JSON.stringify([OPENED]) || JSON.stringify(mergeable) !== JSON.stringify([OTHER])) {
    throw new Error(`expected ${OPENED} alone canonical and Merge on ${OTHER}; Settings shows ${JSON.stringify(listed)}`);
  }
  proof.steps.push(`Settings marks ${OPENED} (the file the day opens) alone canonical and offers Merge on ${OTHER}`);

  await clickMerge(OTHER);
  await waitForFileText(path.join(GRAPH, OPENED), (text) => text.includes("only in iso") && text.includes("only in underscore"), { timeoutMs: 15_000 });
  await browser.waitUntil(() => !fs.existsSync(path.join(GRAPH, OTHER)), {
    timeout: 15_000, interval: 150, timeoutMsg: `${OTHER} is still in the graph after Merge`,
  });
  proof.steps.push(`Merge folds ${OTHER} into ${OPENED} and moves it out of the graph`);

  // The merged day leaves the duplicate list; close Settings and open the day.
  await browser.waitUntil(async () => (await rows()).length === 0, {
    timeout: 15_000, interval: 200, timeoutMsg: "the merged day is still listed as a duplicate",
  });
  await browser.keys("Escape");
  await browser.$(".settings-modal").waitForExist({ timeout: 5000, reverse: true });
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === TITLE
    && await dayShows("only in iso") && await dayShows("only in underscore"), {
    timeout: 15_000, interval: 150, timeoutMsg: `the open day ${TITLE} still shows the pre-merge blocks after Merge`,
  });
  proof.steps.push(`the day already on screen shows both files' blocks after Merge, without navigating`);

  // Reopening it (admission into a fresh view) shows the merged day too.
  await openPageByName(browser, "Notes");
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Notes", { timeout: 15_000, interval: 150 });
  await openPageByName(browser, TITLE);
  await browser.waitUntil(async () => {
    if ((await currentPageTitle(browser)) !== TITLE) return false;
    return browser.execute(() => {
      const text = document.querySelector(".main-content .page-blocks")?.textContent ?? "";
      return text.includes("only in underscore") && text.includes("only in iso");
    });
  }, { timeout: 15_000, interval: 150, timeoutMsg: `the day ${TITLE} does not show both files' blocks after Merge` });
  proof.steps.push(`opening ${TITLE} shows both files' blocks`);
  await saveJourneyScreenshot(browser, path.join(ARTIFACT, "merged-day.png"));
  console.log(JSON.stringify(proof));
} catch (error) {
  proof.error = String(error);
  if (browser) {
    await saveJourneyScreenshot(browser, path.join(ARTIFACT, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(ARTIFACT, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(ARTIFACT, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
