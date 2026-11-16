// Native routed-page journeys for GH #739, #740, #743 and #766.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.resolve("artifacts/parity-09/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, `${process.env.PARITY_PHASE || "after"}-`));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const S = "76600000-0000-4000-8000-000000000001";
const C = "76600000-0000-4000-8000-000000000002";
const sourceFile = path.join(graph, "pages/Source.md");
fs.writeFileSync(sourceFile, `- Source root\n  id:: ${S}\n  collapsed:: true\n  - Source predecessor\n  - Remove me\n    id:: ${C}\n- Source survivor\n`);
fs.writeFileSync(path.join(graph, "pages/Parity host.md"), `- {{embed ((${S}))}}\n- Host survivor\n`);
fs.writeFileSync(path.join(graph, "pages/Research.md"), "- Tag page [[Other]]\n");
fs.writeFileSync(path.join(graph, "pages/Tagged.md"), "tags:: Research\n\n- Tagged body\n");
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- Other body\n");
fs.writeFileSync(path.join(graph, "pages/Query source.md"), "- Root [[Other]]\n  - Matched child\n    type:: parityproject\n");
fs.writeFileSync(path.join(graph, "pages/Query host.md"), "- {{query (property type parityproject)}}\n");
fs.writeFileSync(path.join(graph, "pages/Property-only source.md"), "- Root parent\n  - type:: parityempty\n");
fs.writeFileSync(path.join(graph, "pages/Query table host.md"), "- {{query (property type parityempty)}}\n  tine.view:: table\n");
const port = Number(process.env.E2E_DRIVER_PORT || 4889);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
let browser;
let stage = "startup";
const passed = [];
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
async function shiftClick(selector) {
  await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: "\uE008" }] }]);
  try { await browser.$(selector).click(); } finally { await browser.releaseActions(); }
}
function pass() { passed.push(stage); console.log(`PASS: ${stage}`); }
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1200, 1000);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });

  // Diagnostic only, not an assertion that ancestor omission is desirable.
  // The DB-vs-file-graph table behavior in #744 needs the manager's decision.
  if (process.env.PARITY_ONLY === "744") {
    stage = "#744 current table characterization";
    await openPageByName(browser, "Query table host");
    await browser.$(".main-content .sheet-table").waitForExist({ timeout: 20000 });
    const text = await browser.$(".main-content .sheet-table").getText();
    fs.writeFileSync(path.join(run, "table-text.txt"), text);
    console.log(JSON.stringify({ issue: 744, showsProperty: text.includes("parityempty"), showsAncestor: text.includes("Root parent") }));
    pass();
  }

  if (!process.env.PARITY_ONLY || process.env.PARITY_ONLY === "739") {
    stage = "#739 tagged-page fold survives switching tabs, with count retained";
    await openPageByName(browser, "Research");
    const header = ".main-content .tagged-pages .references-header";
    await browser.$(header).waitForExist({ timeout: 20000 });
    assert((await browser.$(header).getText()).includes("1"));
    await browser.$(header).click();
    assert.equal(await browser.$(header).getAttribute("aria-expanded"), "false");
    await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: "\uE009" }] }]);
    try { await browser.$(".main-content .block-content .page-ref").click(); }
    finally { await browser.releaseActions(); }
    const otherTab = await browser.$('//div[contains(@class,"tab") and ./*[contains(@class,"tab-title") and normalize-space(.)="Other"]]');
    await otherTab.waitForExist();
    await otherTab.click();
    await browser.waitUntil(async () => (await browser.$(".main-content").getText()).includes("Other body"));
    // Use the visible tab label, independent of its position in the strip.
    const point = await browser.execute(() => {
      const label = [...document.querySelectorAll('.tab')].find(el => el.querySelector('.tab-title')?.textContent.trim() === "Research");
      if (!label) return null;
      const r = label.getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
    });
    assert(point, "Research tab remains in the tab strip");
    await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
    ] }]);
    await browser.releaseActions();
    await browser.$(header).waitForExist();
    assert.equal(await browser.$(header).getAttribute("aria-expanded"), "false");
    assert((await browser.$(header).getText()).includes("1"));
    pass();
  }

  if (!process.env.PARITY_ONLY || process.env.PARITY_ONLY === "740") {
    stage = "#740 Shift-click calendar day opens a sidebar journal and retains the main page";
    await openPageByName(browser, "Other");
    await browser.$('button[title="Go to date"]').click();
    await shiftClick(".calendar-jump-pop .dp-cell.today");
    await browser.$(".right-sidebar .ls-block").waitForExist({ timeout: 20000 });
    assert((await browser.$(".main-content").getText()).includes("Other body"));
    assert.equal(await browser.$(".calendar-jump-pop").isExisting(), false);
    assert.equal(await browser.$(".right-sidebar .resource-failure").isExisting(), false);
    pass();
  }

  if (!process.env.PARITY_ONLY || process.env.PARITY_ONLY === "743") {
    stage = "#743 a root link inside a property query result navigates";
    await openPageByName(browser, "Query host");
    const link = '.main-content .query-group .ref-breadcrumb .page-ref';
    await browser.$(link).waitForExist({ timeout: 20000 });
    await browser.$(link).click();
    await browser.waitUntil(async () => (await browser.$(".main-content").getText()).includes("Other body"), { timeout: 15000 });
    pass();
  }

  if (!process.env.PARITY_ONLY || process.env.PARITY_ONLY === "766") {
    stage = "#766 Backspace deletes an emptied embedded child of a folded source; Undo restores it";
    await openPageByName(browser, "Parity host");
    const sourceRow = `.main-content .embed-block [data-block-ref="${S}"]`;
    await browser.$(`${sourceRow} > .block-main .collapse-toggle`).waitForExist({ timeout: 20000 });
    await browser.$(`${sourceRow} > .block-main .collapse-toggle`).click();
    const child = `.main-content .embed-block [data-block-ref="${C}"]`;
    await browser.$(`${child} > .block-main .block-content`).click();
    await browser.$(`${child} textarea.block-editor`).waitForExist();
    await browser.keys(["Control", "a"]);
    await browser.keys(["Backspace"]);
    await browser.keys(["Backspace"]);
    // Merge transfers the emptied child's authored ID to an un-ID'd survivor.
    // Count source children, rather than requiring identity to disappear.
    await waitForFileText(sourceFile, text => (text.match(/^  -(?: |$)/gm) ?? []).length === 1 && text.includes("Source predecessor") && !text.includes("Remove me"));
    assert(fs.readFileSync(sourceFile, "utf8").includes("collapsed:: true"));
    assert(fs.readFileSync(path.join(graph, "pages/Parity host.md"), "utf8").includes(`{{embed ((${S}))}}`));
    await browser.keys(["Control", "z"]);
    await waitForFileText(sourceFile, text => (text.match(/^  -(?: |$)/gm) ?? []).length === 2 && text.includes(`id:: ${C}`));
    pass();
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit, app, passed, graph }, null, 2));
} catch (error) {
  if (browser) {
    try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "fail", testedCommit, app, expected: stage, observed: String(error), classification: "ambiguous", passed, graph }, null, 2));
  console.error(`FAIL ${stage}: ${error}`);
  process.exitCode = 1;
} finally {
  await browser?.deleteSession().catch(() => {});
  driver.kill(); fs.closeSync(log);
}
