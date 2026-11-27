// GH #975: "Edit clock entries" from the block menu corrects one CLOCK row in the
// saved file. Normal editing keeps the LOGBOOK hidden; Undo restores the row.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { editorSelector, editorState, selectEditor } from "./lib/e2e-block-editor.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const ART = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/clock-entries-975"));
fs.mkdirSync(ART, { recursive: true });
const run = fs.mkdtempSync(path.join(ART, "run-"));
const graph = path.join(run, "graph");
for (const d of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, d), { recursive: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, d));
const uuid = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e75";
const row1 = "CLOCK: [2026-11-20 Fri 09:00:00]--[2026-11-20 Fri 10:00:00] =>  01:00:00";
const row2 = "CLOCK: [2026-11-21 Sat 09:00:00]--[2026-11-21 Sat 10:00:00] =>  01:00:00";
const fixedEnd = "10:30:00] =>  01:30:00";
const source = `- TODO write the report\n  id:: ${uuid}\n  collapsed:: true\n  :LOGBOOK:\n  ${row1}\n  ${row2}\n  :END:\n- Witness\n`;
const fixed = source.replace(row1, row1.replace("10:00:00] =>  01:00:00", fixedEnd));
const file = path.join(graph, "pages/Tasks.md");
fs.writeFileSync(file, source);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:preferred-format :markdown}\n");
const port = Number(process.env.E2E_DRIVER_PORT || 4975);
const display = Number(process.env.E2E_X_DISPLAY || 1975);
const env = { ...process.env, TINE_GRAPH: graph, DISPLAY: `127.0.0.1:${display}`,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
// TCP-only and no lock file keep runtime artifacts inside the assigned worktree.
const xvfb = spawn("Xvfb", [`:${display}`, "-screen", "0", "1920x1080x24", "-nolock", "-nolisten", "unix", "-listen", "tcp", "-ac"], { env, stdio: ["ignore", log, log] });
const driver = spawn(process.env.TAURI_DRIVER || "/usr/local/bin/tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
const row = `.main-content .ls-block[data-block-ref="${uuid}"]`;
let browser;
let stage = "startup";
const proof = { base: process.env.TINE_TESTED_BASE || "working tree", steps: [] };

/** Right-click the task's own bullet and choose `label` from the block menu. */
async function menu(label) {
  await browser.waitUntil(async () => {
    if (await browser.$(".ctx-menu").isExisting()) return true;
    const point = await browser.execute(selector => {
      const bullet = document.querySelector(selector);
      if (!bullet) return null;
      bullet.scrollIntoView({ block: "center" });
      const box = bullet.getBoundingClientRect();
      const x = Math.round(box.left + box.width / 2), y = Math.round(box.top + box.height / 2);
      return bullet.contains(document.elementFromPoint(x, y)) ? { x, y } : null;
    }, `${row} > .block-main > .block-controls .bullet-container`);
    if (!point) return false;
    await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", x: point.x, y: point.y },
      { type: "pointerDown", button: 2 }, { type: "pointerUp", button: 2 },
    ] }]);
    await browser.releaseActions();
    return browser.$(".ctx-menu").waitForExist({ timeout: 1500 }).then(() => true, () => false);
  }, { timeout: 15_000, interval: 150, timeoutMsg: "task bullet menu unavailable" });
  await browser.$(`//div[contains(@class,"ctx-item") and normalize-space(.)="${label}"]`).click();
}
const editorOpen = () => browser.$(editorSelector(row)).waitForExist({ timeout: 10_000 });
const editorClosed = () => browser.$(editorSelector(row)).waitForExist({ timeout: 10_000, reverse: true });

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(ROOT, "target/release/tine") } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Tasks");
  await browser.$(row).waitForExist({ timeout: 20_000 });

  stage = "the menu opens the drawer at its place, caret on the first CLOCK row";
  await menu("Edit clock entries");
  await editorOpen();
  const opened = await browser.waitUntil(async () => {
    const state = await editorState(browser, row);
    return state?.focused && state.start === state.value.indexOf(row1) ? state : false;
  }, { timeout: 10_000, timeoutMsg: "editor did not open with the caret on the first CLOCK row" });
  const buffer = `TODO write the report\n:LOGBOOK:\n${row1}\n${row2}\n:END:`;
  if (opened.value !== buffer) throw new Error(`wrong buffer: ${JSON.stringify(opened.value)}`);
  if (opened.kind !== "cm6") throw new Error(`expected the CodeMirror editor, got ${opened.kind}`);
  proof.steps.push(stage);

  stage = "correcting the first row's end time writes exactly that change";
  // Real keys, digits only: WebDriver's X keyboard maps shifted ':' and '>' unreliably.
  const at = opened.value.indexOf("10:00:00] =>  01:00:00");
  await selectEditor(browser, at + 3, at + 5, row); // end minutes
  await browser.keys(["3", "0"]);
  await selectEditor(browser, at + 17, at + 19, row); // duration minutes
  await browser.keys(["3", "0"]);
  await browser.keys(["Escape"]);
  await waitForFileText(file, text => text === fixed);
  await editorClosed();
  proof.steps.push(stage);

  stage = "normal editing hides the drawer again";
  await browser.$(`${row} > .block-main .block-content`).click();
  await editorOpen();
  const normal = await browser.waitUntil(async () => (await editorState(browser, row))?.value ?? false, { timeout: 10_000 });
  if (normal !== "TODO write the report") throw new Error(`normal editor shows hidden text: ${JSON.stringify(normal)}`);
  await browser.keys(["Escape"]);
  await editorClosed();
  if (fs.readFileSync(file, "utf8") !== fixed) throw new Error("normal re-entry changed the file");
  proof.steps.push(stage);

  stage = "Undo restores the old row";
  // Typed keystrokes may form several undo steps; each step only takes the row back toward its old text.
  for (let step = 0; fs.readFileSync(file, "utf8") !== source; step++) {
    if (step === fixedEnd.length) throw new Error("Undo did not restore the old row");
    const before = fs.readFileSync(file, "utf8");
    await browser.keys(["Control", "z"]);
    await waitForFileText(file, text => text !== before);
  }
  proof.steps.push(stage);

  fs.writeFileSync(path.join(run, "final-dom.html"), await browser.getPageSource());
  proof.outcome = "PASS";
  console.log(`PASS GH #975 clock entries: ${run}`);
} catch (error) {
  Object.assign(proof, { outcome: "FAIL", stage, expected: "Edit clock entries changes exactly the corrected CLOCK row; normal editing hides the drawer; Undo restores it", observed: String(error), classification: "ambiguous; inspect driver log and DOM" });
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.getPageSource()); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  xvfb.kill("SIGTERM");
  fs.closeSync(log);
}
