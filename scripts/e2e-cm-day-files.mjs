// CM step 2 (CM-DAYFILE, D-20261111-CMR14; CM-ORDER, D-20261110-CM;
// UI-CM-DAY-FILE-BOTH): one day held by two files, `journals/2026_11_02.md`
// and the date-named page file `pages/2026-11-02.md`. Opening the day shows
// the journals file; the conflict overview lists the day as one name held by
// both files, the journals file first (the opening file); viewing writes,
// moves and merges nothing. When the journals file goes away (an external
// delete), the same day opens the pages file where it lies.
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
const TMP = fs.mkdtempSync("/tmp/tine-cm-day-files-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4752);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
const JOURNAL = "journals/2026_11_02.md";
const PAGE = "pages/2026-11-02.md";
const TITLE = "Nov 2nd, 2026";
const files = {
  "logseq/config.edn": "{:tine/guide-announced? true}\n",
  [JOURNAL]: "- written in the journal\n",
  [PAGE]: "- written in the date-named page file\n",
  "pages/Notes.md": "- see [[2026-11-02]]\n",
};
for (const [rel, text] of Object.entries(files)) fs.writeFileSync(path.join(GRAPH, rel), text);
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACT, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, graph: GRAPH, steps: [] };

/** The main view's page text. */
const dayText = () => browser.execute(() => document.querySelector(".main-content .page-blocks")?.textContent ?? "");

/** Open the day and wait until it shows `wanted` and not `unwanted`. */
async function openDayShowing(wanted, unwanted, what) {
  await openPageByName(browser, TITLE);
  let text = "";
  try {
    await browser.waitUntil(async () => {
      text = await dayText();
      return (await currentPageTitle(browser)) === TITLE && text.includes(wanted) && !text.includes(unwanted);
    }, { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`${TITLE} should open ${what}; the page shows ${JSON.stringify(text)}`);
  }
}

/** The graph's files and bytes, for the nothing-written check. */
function snapshot() {
  const out = {};
  for (const dir of ["pages", "journals"]) {
    for (const name of fs.readdirSync(path.join(GRAPH, dir)).sort()) out[`${dir}/${name}`] = fs.readFileSync(path.join(GRAPH, dir, name), "utf8");
  }
  return out;
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  const before = snapshot();

  await openDayShowing("written in the journal", "written in the date-named page file", JOURNAL);
  proof.steps.push(`${TITLE} opens ${JOURNAL}, not ${PAGE}`);

  // The conflict overview, reached as the user does: the sidebar's badge.
  await browser.waitUntil(() => browser.execute(() => {
    const badge = document.querySelector(".conflict-queue-badge");
    if (!(badge instanceof HTMLElement)) return false;
    badge.click();
    return true;
  }), { timeout: 20_000, interval: 200, timeoutMsg: "the conflict badge never appeared for the day held by two files" });
  let rows = [];
  try {
    await browser.waitUntil(async () => {
      rows = await browser.execute(() => [...document.querySelectorAll(".conflict-overview .conflict-overview-row")].map((row) => ({
        name: (row.querySelector(".conflict-overview-open")?.textContent ?? "").trim(),
        files: [...row.querySelectorAll(".conflict-overview-name-files button")].map((button) => (button.textContent ?? "").trim()),
      })));
      return rows.some((row) => row.name === TITLE && row.files.length === 2);
    }, { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`the conflict overview should list ${TITLE} with its two files; it lists ${JSON.stringify(rows)}`);
  }
  const row = rows.find((candidate) => candidate.name === TITLE);
  proof.overview = row;
  if (!row.files[0].startsWith(JOURNAL) || !row.files[1].startsWith(PAGE)) {
    throw new Error(`the overview should list ${JOURNAL} first (the opening file) and then ${PAGE}; it lists ${JSON.stringify(row.files)}`);
  }
  proof.steps.push(`the conflict overview lists ${TITLE} with ${JOURNAL} (opening file) and ${PAGE}`);
  await saveJourneyScreenshot(browser, path.join(ARTIFACT, "day-files-overview.png"));

  // Viewing changed nothing: no file moved, merged or rewritten.
  for (const [rel, text] of Object.entries(before)) await waitForFileText(path.join(GRAPH, rel), (now) => now === text);
  if (JSON.stringify(snapshot()) !== JSON.stringify(before)) throw new Error(`viewing changed the graph: ${JSON.stringify(snapshot())}`);
  proof.steps.push("viewing the day and the overview wrote, moved and merged nothing");

  // An external delete of the journals file: the pages file now holds the day.
  fs.unlinkSync(path.join(GRAPH, JOURNAL));
  await openPageByName(browser, "Notes");
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Notes", { timeout: 15_000, interval: 150 });
  await openDayShowing("written in the date-named page file", "written in the journal", PAGE);
  await waitForFileText(path.join(GRAPH, PAGE), (now) => now === files[PAGE]);
  if (fs.existsSync(path.join(GRAPH, JOURNAL))) throw new Error(`opening the day recreated ${JOURNAL}`);
  proof.steps.push(`with ${JOURNAL} gone, ${TITLE} opens ${PAGE} where it lies, and no journals file is created`);
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
