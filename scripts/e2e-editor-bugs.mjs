// GH #683/#684 durable native journeys; GH #530 independent X11 paste target.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "evidence/editor-bugs/native"));
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
  const collapsed = checked.replace("- > [!TIP]", `- collapsed:: true${newline}  > [!TIP]`);
  await saved(collapsed);
  if (await browser.execute(() => document.querySelector(".main-content .callout").textContent.includes("collapsed::"))) throw new Error("Metadata leaked into the callout");
  await openPageByName(browser, "Other");
  await openPageByName(browser, "Operations");
  const reopenedQuoteId = await browser.execute(() => document.querySelector(".main-content .callout").closest(".ls-block").getAttribute("data-block-id"));
  if (await browser.$(`[data-block-id="${reopenedQuoteId}"] .ls-block`).isDisplayed()) throw new Error("Collapsed child became visible on reopen");
  await browser.$(`[data-block-id="${reopenedQuoteId}"] .collapse-toggle`).click();
  await saved(checked);
  proof.observations.push("Collapse/expand and route reopen preserve every unrelated file byte");
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
