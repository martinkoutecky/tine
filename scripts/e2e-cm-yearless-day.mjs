// CM SPEC §2.1 (approved label limitation), REG-CM-YEARLESS-DAY-KEY review
// item 2, UI-CM-YEARLESS-DAY-OPENS-LABEL-PAGE: under the year-less title
// format MM-dd the label `11-02` names the ordinary page pages/11-02.md, and
// each day is its own page, displayed by that label. A literal native click
// on [[2026-11-02]] and a calendar pick of 2 November 2026 must open the 2026
// day with its body (not the label page, not the 2025 day); [[11-02]] opens
// the label page; navigating creates no file and changes no bytes.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-cm-yearless-day-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4736);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(TMP, dir), { mode: 0o700 });
fs.mkdirSync(ARTIFACT, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"),
  "{:journal/page-title-format \"MM-dd\"\n :tine/guide-announced? true}\n");
const files = {
  "journals/2025_11_02.md": "- Body of the 2025 day\n",
  "journals/2026_11_02.md": "- Body of the 2026 day\n",
  "pages/11-02.md": "- Body of the label page\n",
  "pages/Source.md": "- Day [[2026-11-02]]\n- Label [[11-02]]\n",
};
const original = Object.keys(files).sort();
for (const [rel, text] of Object.entries(files)) fs.writeFileSync(path.join(GRAPH, rel), text);
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"), XDG_STATE_HOME: path.join(TMP, "state"),
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

const BODIES = ["Body of the 2025 day", "Body of the 2026 day", "Body of the label page"];

/** The routed page is titled `title` and shows `body`, and no other fixture body. */
async function expectShown(what, title, body) {
  const shown = () => browser.execute(() => document.querySelector(".main-content .page-blocks")?.textContent ?? "");
  try {
    await browser.waitUntil(async () => {
      if ((await currentPageTitle(browser)) !== title) return false;
      const text = await shown();
      return text.includes(body) && BODIES.every((other) => other === body || !text.includes(other));
    }, { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`${what} opened ${JSON.stringify(await currentPageTitle(browser))} showing ${JSON.stringify((await shown()).slice(0, 200))}; expected ${JSON.stringify(title)} with ${JSON.stringify(body)}`);
  }
  proof.steps.push(`${what} opens ${title} with ${JSON.stringify(body)}`);
}

async function nativeClick(selector) {
  const point = await browser.waitUntil(() => browser.execute((css) => {
    const node = typeof css === "string" ? document.querySelector(css) : null;
    if (!node) return false;
    const rect = node.getBoundingClientRect();
    const x = Math.round(rect.x + rect.width / 2), y = Math.round(rect.y + rect.height / 2);
    return node.contains(document.elementFromPoint(x, y)) ? { x, y } : false;
  }, selector), { timeout: 15_000, interval: 150, timeoutMsg: `no clickable ${selector}` });
  await browser.performActions([
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
  ]);
  await browser.releaseActions();
}

/** Open the calendar and pick 2 November 2026, stepping months as needed. */
async function pickNovemberSecond() {
  await nativeClick('button[title="Go to date"]');
  await browser.$(".calendar-jump-pop").waitForExist({ timeout: 10_000 });
  for (let i = 0; i < 48; i++) {
    const title = await browser.$(".calendar-jump-pop .dp-title").getText();
    const [month, year] = title.trim().split(/\s+/);
    const months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const delta = (2026 - Number(year)) * 12 + (10 - months.indexOf(month));
    if (delta === 0) break;
    await browser.$$(".calendar-jump-pop .dp-nav")[delta < 0 ? 0 : 1].click();
  }
  await browser.waitUntil(() => browser.execute(() => {
    const cells = [...document.querySelectorAll(".calendar-jump-pop .dp-cell:not(.dp-empty)")];
    const index = cells.findIndex((node) => node.textContent.trim() === "2");
    if (index < 0) return false;
    cells[index].setAttribute("data-e2e-pick", "1");
    return true;
  }), { timeout: 10_000, interval: 150, timeoutMsg: "no calendar cell 2" });
  await nativeClick('.calendar-jump-pop .dp-cell[data-e2e-pick="1"]');
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Source");
  await clickLink("2026-11-02");
  await expectShown("[[2026-11-02]]", "11-02", "Body of the 2026 day");
  await openPageByName(browser, "Source");
  await clickLink("11-02");
  await expectShown("[[11-02]]", "11-02", "Body of the label page");
  await openPageByName(browser, "Source");
  await pickNovemberSecond();
  await expectShown("the calendar's 2 November 2026", "11-02", "Body of the 2026 day");
  const listed = (dir) => fs.readdirSync(path.join(GRAPH, dir)).sort().map((name) => `${dir}/${name}`);
  const now = [...listed("journals"), ...listed("pages")].sort();
  if (JSON.stringify(now) !== JSON.stringify(original)) throw new Error(`navigation changed the file set: ${now}`);
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
