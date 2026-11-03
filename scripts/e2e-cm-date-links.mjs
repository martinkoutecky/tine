// CM step 2 (conceptual model Q3/Q8): what a rendered [[link]] opens, by a
// literal native click, in a graph with a custom :journal/file-name-format.
// UI-CM-FILE-FORMAT-DATE-LINK-BLANK: [[22-10-2026]] (the journal's file-name
// spelling) opened a blank ordinary page. UI-CM-JOURNAL-ALIAS-LINK-BLANK: a
// link to a journal's alias opened a blank page named like the journal.
// Both destinations must be the journal itself, with its body, and no file
// may be created by navigating.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-cm-date-links-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4734);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"),
  "{:journal/file-name-format \"dd-MM-yyyy\"\n :journal/page-title-format \"MMM do, yyyy\"\n :tine/guide-announced? true}\n");
const files = {
  "journals/22-10-2026.md": "- Journal body of the twenty-second\n",
  "journals/26-10-2026.md": "alias:: launch day\n\n- Launch journal body\n",
  "pages/Source.md": "- File spelling [[22-10-2026]]\n- Alias [[launch day]]\n",
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

/** Native click on the rendered link whose text is `text` in the main pane. */
async function clickLink(text) {
  const point = await browser.waitUntil(() => browser.execute((wanted) => {
    const undecorate = (value) => (value ?? "").trim().replace(/^\[\[/, "").replace(/\]\]$/, "").trim();
    const link = [...document.querySelectorAll(".main-content .page-blocks a.page-ref")]
      .find((node) => undecorate(node.textContent) === wanted);
    if (!link) return false;
    link.scrollIntoView({ block: "center" });
    const rect = link.getClientRects()[0];
    const x = Math.round(rect.x + rect.width / 2), y = Math.round(rect.y + rect.height / 2);
    if (!link.contains(document.elementFromPoint(x, y))) return false;
    return { x, y };
  }, text), { timeout: 15_000, interval: 150, timeoutMsg: `no clickable rendered link ${JSON.stringify(text)}` });
  await browser.performActions([
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
  ]);
  await browser.releaseActions();
}

async function expectJournal(link, title, body) {
  await openPageByName(browser, "Source");
  await clickLink(link);
  try {
    await browser.waitUntil(async () => {
      if ((await currentPageTitle(browser)) !== title) return false;
      return browser.execute((text) => (document.querySelector(".main-content .page-blocks")?.textContent ?? "").includes(text), body);
    }, { timeout: 15_000, interval: 150 });
  } catch {
    const shown = await browser.execute(() => document.querySelector(".main-content .page-blocks")?.textContent ?? "");
    throw new Error(`[[${link}]] opened ${JSON.stringify(await currentPageTitle(browser))} showing ${JSON.stringify(shown.slice(0, 200))}; expected the journal ${JSON.stringify(title)} with ${JSON.stringify(body)}`);
  }
  proof.steps.push(`[[${link}]] opens the journal ${title} with its body`);
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await expectJournal("22-10-2026", "Oct 22nd, 2026", "Journal body of the twenty-second");
  await expectJournal("launch day", "Oct 26th, 2026", "Launch journal body");
  const listed = (dir) => fs.readdirSync(path.join(GRAPH, dir)).sort();
  if (JSON.stringify(listed("pages")) !== JSON.stringify(["Source.md"])) throw new Error(`navigation created pages: ${listed("pages")}`);
  if (JSON.stringify(listed("journals")) !== JSON.stringify(["22-10-2026.md", "26-10-2026.md"])) throw new Error(`navigation changed journals: ${listed("journals")}`);
  for (const [rel, text] of Object.entries(files)) {
    if (fs.readFileSync(path.join(GRAPH, rel), "utf8") !== text) throw new Error(`navigation changed ${rel}`);
  }
  proof.steps.push("navigation creates no file and leaves fixture bytes unchanged");
  console.log(JSON.stringify(proof));
} catch (error) {
  proof.error = String(error);
  if (browser) {
    await browser.saveScreenshot(path.join(ARTIFACT, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(ARTIFACT, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(ARTIFACT, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
