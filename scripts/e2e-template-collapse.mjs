import os from "node:os";
// GH #670: native slash-template insertion, save, and cold reopen.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(os.tmpdir(), "tine-e2e-gh670"));
fs.mkdirSync(artifacts, { recursive: true });
const TMP = fs.mkdtempSync(path.join(artifacts, "run-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const template = "- Demo\n  template:: collapse-demo\n  template-including-parent:: false\n  - Folded\n    collapsed:: true\n    - hidden child\n  - Expanded\n    collapsed:: false\n    - visible false child\n  - Default\n    - visible default child\n";
fs.writeFileSync(path.join(GRAPH, "pages/Templates.md"), template);
fs.writeFileSync(path.join(GRAPH, "pages/Target.md"), "- insert here\n");
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4710);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
const testedBase = "55b4055edb88476eb458776e398375fbfea9c975 + gh670 working diff";
const connect = () => remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
  capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
async function observe() {
  await browser.waitUntil(() => browser.execute(() => {
    const text = [...document.querySelectorAll(".block-content")].map((node) => node.textContent).join("\n");
    return text.includes("Folded") && text.includes("visible false child") && text.includes("visible default child") && !text.includes("hidden child");
  }), { timeout: 10_000, timeoutMsg: "inserted collapse state is not visible" });
}
try {
  browser = await connect();
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Target");
  stage = "template entry";
  const target = await browser.$("//div[contains(@class, 'block-content') and normalize-space(.) = 'insert here']");
  await target.waitForDisplayed({ timeout: 10_000 });
  await target.click();
  const editor = await browser.$("textarea.block-editor");
  await editor.waitForExist({ timeout: 10_000 });
  // Set text and dispatch its input atomically: WebDriver clearValue blurs this
  // controlled textarea before its subsequent elementSendKeys call on WebKit.
  await browser.execute(() => {
    const textarea = document.querySelector("textarea.block-editor");
    textarea.value = "/collapse-demo";
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "/collapse-demo" }));
    textarea.focus();
  });
  await browser.$(".autocomplete .ac-label").waitForExist({ timeout: 10_000 });
  await browser.keys(["Enter"]);
  await browser.keys(["Escape"]);
  stage = "live visibility and save";
  await observe();
  const saved = path.join(GRAPH, "pages/Target.md");
  await browser.waitUntil(() => fs.readFileSync(saved, "utf8").includes("collapsed:: true"), { timeout: 10_000 });
  const savedText = fs.readFileSync(saved, "utf8");
  for (const text of ["hidden child", "visible false child", "visible default child", "collapsed:: false"])
    if (!savedText.includes(text)) throw new Error(`saved template lost ${text}`);
  await browser.deleteSession();
  browser = await connect();
  stage = "cold reopen";
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Target");
  await observe();
  if (fs.readFileSync(path.join(GRAPH, "pages/Templates.md"), "utf8") !== template) throw new Error("template source changed");
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify({ testedBase, app: APP, outcome: "PASS: collapsed insertion, persisted children, cold reopen, source unchanged" }));
  console.log(`PASS GH #670: ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(TMP, "failure.txt"), `Tested: ${testedBase}\nJourney: slash insert, save, reopen. Stage: ${stage}. Expected: folded children hidden; expanded children visible. Observed: ${error.stack}\nClassification: ambiguous until driver log and DOM inspected.\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
