// GH #683/#684/#721 durable native journeys; GH #530 independent X11 paste target.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/editor-bugs-native"));
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const invalidConfig = process.env.E2E_INVALID_CONFIG === "1";
const configText = invalidConfig ? "{:graph-view {:layout : grid}}\n" : "{}\n";
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), configText);
const newline = process.env.E2E_LINE_ENDING === "crlf" ? "\r\n" : "\n";
const original = ["- Tasks", "  * [ ] first", "  * [ ] second", "- > [!TIP]", "  > helpful", "  ", "  id:: 11111111-1111-4111-8111-111111111111", "  - child", "- unrelated marker", ""].join(newline);
const file = path.join(GRAPH, "pages/Operations.md");
fs.writeFileSync(file, original);
fs.writeFileSync(path.join(GRAPH, "pages/Other.md"), "- other page\n");
// GH #721: Logseq Android writes `collapsed:: true` after a block's last line, which the
// parser folds into a final quote. Repeated collapse must neither stack rows nor show them.
const androidFile = path.join(GRAPH, "pages/Android.md");
fs.writeFileSync(androidFile, ["- Outside", "  > Something", "  collapsed:: true", "\t- Inside", ""].join(newline));
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4896);
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver");
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), app: APP,
  binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), observations: [] };
async function saved(expected) {
  await browser.waitUntil(() => fs.readFileSync(file, "utf8") === expected, { timeout: 15000, timeoutMsg: "Saved bytes differ from the expected token/property edit" });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Operations");
  if (invalidConfig) {
    const text = await browser.$(".main-content").getText();
    if (!text.includes("Tasks") || !text.includes("helpful") || !text.includes("unrelated marker")) throw new Error("Invalid config hid existing page contents");
    if (fs.readFileSync(file, "utf8") !== original || fs.readFileSync(path.join(GRAPH, "logseq/config.edn"), "utf8") !== configText) throw new Error("Invalid config or page bytes changed");
    proof.observations.push("Exact malformed graph-view EDN keeps page content visible and file bytes unchanged");
  } else {
  const boxes = await browser.$$(".main-content .block-checkbox");
  await boxes[1].click();
  if ((await browser.$$("textarea.block-editor")).length) throw new Error("Checkbox entered edit mode");
  const checked = original.replace("* [ ] second", "* [x] second");
  await saved(checked);
  proof.observations.push("Click ticks only the second item, through a durable save");
  const quoteId = await browser.execute(() => document.querySelector(".main-content .callout").closest(".ls-block").getAttribute("data-block-id"));
  await browser.$(`[data-block-id="${quoteId}"] .collapse-toggle`).click();
  // OG reads only the property group nearest the bullet (graph_parser/block.cljs extract-blocks),
  // so collapsed:: joins the existing id:: group instead of opening a second one above the quote.
  const idRow = "  id:: 11111111-1111-4111-8111-111111111111";
  const collapsed = checked.replace(idRow, `${idRow}${newline}  collapsed:: true`);
  await saved(collapsed);
  if (await browser.execute(() => document.querySelector(".main-content .callout").textContent.includes("collapsed::"))) throw new Error("Metadata leaked into the callout");
  await openPageByName(browser, "Other");
  await openPageByName(browser, "Operations");
  const reopenedQuoteId = await browser.execute(() => document.querySelector(".main-content .callout").closest(".ls-block").getAttribute("data-block-id"));
  if (await browser.$(`[data-block-id="${reopenedQuoteId}"] .ls-block`).isDisplayed()) throw new Error("Collapsed child became visible on reopen");
  await browser.$(`[data-block-id="${reopenedQuoteId}"] .collapse-toggle`).click();
  await saved(checked);
  proof.observations.push("Collapse/expand and route reopen preserve every unrelated file byte");
  // GH #721: five collapse/expand taps on a block whose trailing `collapsed::` row sits in its quote.
  await openPageByName(browser, "Android");
  const androidRows = () => fs.readFileSync(androidFile, "utf8").split(/\r?\n/).filter(l => /^\s*collapsed::/i.test(l)).length;
  const outsideToggle = async () => {
    const id = await browser.execute(() => [...document.querySelectorAll(".main-content .block-content")].find(el => el.textContent.startsWith("Outside")).closest(".ls-block").getAttribute("data-block-id"));
    return browser.$(`[data-block-id="${id}"] .collapse-toggle`);
  };
  for (let tap = 1; tap <= 5; tap++) {
    await (await outsideToggle()).click();
    const expected = tap % 2; // taps 1, 3, 5 collapse (row present); 2, 4 expand (row removed)
    await browser.waitUntil(() => androidRows() === expected, { timeout: 15000, timeoutMsg: `tap ${tap}: expected ${expected} collapsed:: row(s), file has ${androidRows()}` });
    await browser.pause(700); // a stacking save would land here
    if (androidRows() !== expected) throw new Error(`tap ${tap}: collapsed:: rows stacked to ${androidRows()}`);
    if (await browser.execute(() => document.querySelector(".main-content").textContent.includes("collapsed::"))) throw new Error(`tap ${tap}: collapsed:: shown as text`);
    if (tap === 1) {
      // Follow-up F1: the first collapse moves the swallowed row before the quote, where the parser
      // reads it, so the collapse survives a restart (a true -> true rewrite left it swallowed).
      const expectedFile = ["- Outside", "  collapsed:: true", "  > Something", "\t- Inside", ""].join(newline);
      await browser.waitUntil(() => fs.readFileSync(androidFile, "utf8") === expectedFile, { timeout: 15000, timeoutMsg: `tap 1: the collapse row did not move before the quote: ${JSON.stringify(fs.readFileSync(androidFile, "utf8"))}` });
    }
  }
  const androidBytes = fs.readFileSync(androidFile, "utf8");
  if (!androidBytes.includes("Something") || !androidBytes.includes("Inside")) throw new Error("Quote or child lost");
  proof.observations.push("Five taps on a block whose collapsed:: row trails its quote keep at most one row and never show it");
  await openPageByName(browser, "Operations");
  await browser.$(".main-content .block-checkbox").click(); // restore first item afterward
  await saved(checked.replace("* [ ] first", "* [x] first"));
  await browser.$(".main-content .block-checkbox").click();
  await saved(checked);
  await browser.execute(() => [...document.querySelectorAll(".main-content .block-content")].find(el => el.textContent.startsWith("Tasks")).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
  await browser.$("textarea.block-editor").waitForExist({ timeout: 10000 });
  await browser.keys(["Control", "Home"]);
  for (let i = 0; i < 5; i++) await browser.keys(["Shift", "ArrowRight"]);
  await browser.waitUntil(() => browser.execute(() => { const e = document.querySelector("textarea.block-editor"); return e.value.slice(e.selectionStart, e.selectionEnd) === "Tasks"; }), { timeout: 5000 });
  const selection = JSON.parse(execFileSync("python3", ["scripts/read-primary-selection.py"], { env, encoding: "utf8" }));
  proof.primarySelection = selection;
  if (selection.text !== "Tasks" || (selection.html && (selection.html.includes("unrelated marker") || selection.html.includes("helpful")))) throw new Error("PRIMARY contains text outside the selected passage");
  proof.observations.push("Independent X11 paste client receives only the selected text");
  await openPageByName(browser, "Other");
  await openPageByName(browser, "Operations");
  const passage = await browser.execute(() => {
    const body = [...document.querySelectorAll(".main-content .block-content")].find(el => el.textContent.startsWith("Tasks"));
    const leaf = body.querySelector("[data-so]");
    const rect = leaf.getBoundingClientRect();
    return { start: Math.round(rect.left + rect.width * 0.2), end: Math.round(rect.left + rect.width * 0.8), y: Math.round(rect.top + rect.height / 2) };
  });
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, origin: "viewport", x: passage.start, y: passage.y },
    { type: "pointerDown", button: 0 },
    { type: "pointerMove", duration: 300, origin: "viewport", x: passage.end, y: passage.y },
    { type: "pointerUp", button: 0 },
  ] }]);
  const dragged = await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    return editor ? editor.value.slice(editor.selectionStart, editor.selectionEnd) : window.getSelection()?.toString();
  });
  if (!dragged || !"Tasks".includes(dragged)) throw new Error("Mouse drag did not select a bounded passage");
  const dragSelection = JSON.parse(execFileSync("python3", ["scripts/read-primary-selection.py"], { env, encoding: "utf8" }));
  proof.dragSelection = { expected: dragged, ...dragSelection };
  if (dragSelection.text !== dragged || (dragSelection.html && (dragSelection.html.includes("unrelated marker") || dragSelection.html.includes("helpful")))) throw new Error("Drag PRIMARY contains text outside the selected passage");
  proof.observations.push("Mouse-drag PRIMARY plain/HTML contain only the selected passage");
  // GH #530: a selection that ends in the LAST text node of the editor (every
  // single-line block, any select-all) made WebKitGTK serialize text/html from the
  // textarea to the end of the document: editing markup, toolbar buttons, later blocks.
  const primaryFor = async (startsWith, label, forbidden) => {
    await browser.keys(["Escape"]);
    await browser.waitUntil(async () => (await browser.$$("textarea.block-editor")).length === 0, { timeout: 5000 });
    await browser.execute((prefix) => [...document.querySelectorAll(".main-content .block-content")].find(el => el.textContent.startsWith(prefix)).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })), startsWith);
    await browser.$("textarea.block-editor").waitForExist({ timeout: 10000 });
    await browser.keys(["Control", "a"]);
    const expected = await browser.execute(() => { const e = document.querySelector("textarea.block-editor"); return e.value.slice(e.selectionStart, e.selectionEnd); });
    await browser.$(".sel-toolbar").waitForExist({ timeout: 5000 });
    const got = JSON.parse(execFileSync("python3", ["scripts/read-primary-selection.py"], { env, encoding: "utf8" }));
    proof[label] = { expected, ...got, html: got.html && got.html.slice(0, 400) };
    if (got.text !== expected) throw new Error(`${label}: PRIMARY text differs from the selection`);
    for (const bad of ["<textarea", "contenteditable", "data-selection-action", ...forbidden]) {
      if (got.html && got.html.includes(bad)) throw new Error(`${label}: PRIMARY text/html carries ${bad}`);
    }
    proof.observations.push(`${label}: select-all PRIMARY plain/HTML carry only the block text`);
  };
  await primaryFor("unrelated marker", "singleLineSelectAll", []);
  await primaryFor("Tasks", "multiLineSelectAll", ["unrelated marker"]);
  await browser.keys(["Escape"]);
  }
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect native log and DOM";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
