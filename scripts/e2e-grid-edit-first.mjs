// GH #874: the reporter's caret journey on a routed page, through the real app/save path.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { editorKind, editorSelector, editorText, scratchProfile } from "./lib/native-editor-bench.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.join(ROOT, "artifacts/grid-edit-first-874");
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, `native-${editorKind}-`));
const GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
const FILE = path.join(GRAPH, "pages/Grid edit first.md");
fs.writeFileSync(FILE, "- Before\n- Grid host\n  tine.view:: grid\n  tine.col-widths:: 0=260;1=180\n  -\n    - alpha\n    - beta\n  -\n    - gamma\n    - delta\n- After\n");
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const profile = scratchProfile(RUN);
const fixture = accountHomeFixture(RUN, RUN);
const env = { ...fixture.env, ...profile, TINE_GRAPH: GRAPH, GDK_BACKEND: "x11",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1" };
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4874);
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const driver = spawn(TD, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
let browser, stage = "startup";
const evidence = [];
const screenshot = name => execFileSync("import", ["-window", "root", path.join(RUN, name)], { timeout: 10000 });
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 3, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  await openPageByName(browser, "Grid edit first");
  await browser.$(".sheet-grid").waitForExist({ timeout: 15000 });
  const gridId = await browser.$(".sheet-grid").getAttribute("data-sheet-grid-id");
  const textIs = async expected => browser.waitUntil(async () => {
    const editor = await browser.$(editorSelector);
    return await editor.isExisting() && await editorText(browser) === expected;
  }, { timeout: 10000, timeoutMsg: `expected Edit on ${JSON.stringify(expected)}` });
  const end = () => browser.keys(["Control", "End", "NULL"]);
  const recordCell = async label => {
    const state = await browser.execute(() => {
      const editor = document.querySelector("textarea.block-editor, .cm-content[contenteditable=true]");
      const cell = editor?.closest(".sheet-cell");
      return { focused: !!editor && (document.activeElement === editor || editor.contains(document.activeElement)),
        row: cell?.dataset.row, col: cell?.dataset.col };
    });
    assert.equal(state.focused, true, `${label}: caret remains in Edit`);
    evidence.push({ label, ...state });
    return state;
  };

  stage = "Down from grid host into top-left";
  await browser.$(`.ls-block[data-block-id="${gridId}"] > .block-main .block-content`).click();
  await browser.$(editorSelector).waitForExist({ timeout: 10000 });
  await end();
  await browser.keys(["ArrowDown"]);
  await textIs("alpha");
  assert.deepEqual(await recordCell(stage), { focused: true, row: "0", col: "0" });

  stage = "across cells and Right at row end";
  await end();
  await browser.keys(["ArrowRight"]);
  await textIs("beta");
  await end();
  await browser.keys(["ArrowRight"]);
  await textIs("gamma");
  assert.deepEqual(await recordCell(stage), { focused: true, row: "1", col: "0" });
  await browser.keys(["Tab"]);
  await browser.keys(["Shift", "Tab", "NULL"]);
  // Tab to delta, then Shift+Tab back to gamma; both remain in Edit.
  await textIs("gamma");

  stage = "Enter and Tab indent inside a cell";
  await end();
  await browser.keys(["Enter"]);
  await textIs("");
  await browser.keys("first child");
  await browser.keys(["Enter"]);
  await textIs("");
  await browser.keys("second child");
  await browser.keys(["Tab"]);
  await textIs("second child");
  await recordCell(stage);
  await waitForFileText(FILE, text => /- gamma\n\s+- first child\n\s+- second child/.test(text));
  const nested = await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor, .cm-content[contenteditable=true]");
    const line = editor?.closest(".sheet-nested-line");
    return !!line?.parentElement?.closest(".sheet-nested-line");
  });
  assert.equal(nested, true, "Tab nests the second child under the first inside gamma");
  screenshot("nested-edit.png");

  stage = "Esc keeps text and enters Cell Selection";
  await browser.keys(" kept");
  await browser.keys(["Escape"]);
  await browser.$(editorSelector).waitForExist({ reverse: true, timeout: 10000 });
  assert.equal(await browser.$('.sheet-cell-selected[data-row="1"][data-col="0"]').isExisting(), true);
  const saved = await waitForFileText(FILE, text => text.includes("second child kept"));
  assert.match(saved, /- gamma\n\s+- first child\n\s+- second child kept/);
  assert.match(saved, /- alpha\n\s+- beta/);
  assert.match(saved, /- delta/);
  await browser.keys(["Escape"]);
  assert.equal(await browser.$(`.ls-block[data-block-id="${gridId}"] > .block-main.selected`).isExisting(), true);
  screenshot("cell-selection.png");
  fixture.verify();
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ outcome: "PASS", testedCommit, candidate: "worktree diff", app: APP, editorKind, evidence, saved }, null, 2));
  console.log(`PASS ${RUN}`);
} catch (error) {
  try { screenshot("failure.png"); } catch {}
  fs.writeFileSync(path.join(RUN, "failure.txt"), `Tested: ${testedCommit} + worktree diff\nJourney: ${stage}\nExpected: caret enters and walks the grid, Tab indents within a cell, Esc keeps saved text\nObserved: ${error.stack}\nClassification: ambiguous until diagnosed (product assertion or harness/infrastructure)\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close?.();
}
