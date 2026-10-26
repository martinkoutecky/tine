import os from "node:os";
// GitHub discussion #729: a template:: in a page's property header is listed by the slash menu and inserts its header block.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(os.tmpdir(), "tine-e2e-gh729"));
fs.mkdirSync(artifacts, { recursive: true });
const TMP = fs.mkdtempSync(path.join(artifacts, "run-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const usual = "tags::  \nlink:: \ncreate-date:: <% today %>\ntemplate:: Usual note\n";
const bulleted = "- tags::  \n  template:: Bulleted note\n  mood:: calm\n";
fs.writeFileSync(path.join(GRAPH, "pages/Usual note.md"), usual);
fs.writeFileSync(path.join(GRAPH, "pages/Bulleted note.md"), bulleted);
fs.writeFileSync(path.join(GRAPH, "pages/Target.md"), "- insert here\n");
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4730);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
const testedBase = process.env.TINE_TESTED_BASE || "release/0.8 batch/fix-beta7 (gh729 backport of og 007cbc245)";
const connect = () => remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
  capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
async function typeSlash(query) {
  await browser.execute((text) => {
    const textarea = document.querySelector("textarea.block-editor");
    textarea.value = text;
    textarea.setSelectionRange(text.length, text.length);
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
    textarea.focus();
  }, query);
}
const menuLabels = () => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")].map((n) => n.textContent.trim()));
try {
  browser = await connect();
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Target");
  stage = "slash menu lists the header template";
  const target = await browser.$("//div[contains(@class, 'block-content') and normalize-space(.) = 'insert here']");
  await target.waitForDisplayed({ timeout: 10_000 });
  await target.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 10_000 });
  // Set text and dispatch its input atomically: WebDriver clearValue blurs this
  // controlled textarea before its subsequent elementSendKeys call on WebKit.
  await typeSlash("/template");
  await browser.waitUntil(async () => {
    const labels = await menuLabels();
    return labels.some((l) => l.includes("Usual note")) && labels.some((l) => l.includes("Bulleted note"));
  }, { timeout: 10_000, timeoutMsg: `/template did not list the page-header template: ${JSON.stringify(await menuLabels())}` });
  stage = "insert";
  await typeSlash("/usual");
  await browser.waitUntil(async () => (await menuLabels()).some((l) => l.includes("Usual note")), { timeout: 10_000, timeoutMsg: "/usual did not list Usual note" });
  // Pick the template row itself: the fuzzy menu may rank a command above it.
  const labels = await menuLabels();
  const at = labels.findIndex((l) => l.includes("Usual note"));
  if (at < 0) throw new Error(`no Usual note row in ${JSON.stringify(labels)}`);
  for (let i = 0; i < at; i++) await browser.keys(["ArrowDown"]);
  await browser.keys(["Enter"]);
  await browser.keys(["Escape"]);
  stage = "saved header block";
  const saved = path.join(GRAPH, "pages/Target.md");
  await browser.waitUntil(() => fs.readFileSync(saved, "utf8").includes("create-date::"), { timeout: 10_000, timeoutMsg: "template was not inserted into Target" });
  const savedText = fs.readFileSync(saved, "utf8");
  // A properties-only first block is saved as the page's own property header (no bullet), as for any such block.
  if (!/^(- )?tags::/m.test(savedText) || !/^\s*link::/m.test(savedText)) throw new Error(`inserted header lost its property lines:\n${savedText}`);
  if (!/create-date:: \[\[[A-Z][a-z]{2} \d{1,2}(st|nd|rd|th), \d{4}\]\]/.test(savedText)) throw new Error(`<% today %> was not expanded:\n${savedText}`);
  if (/template::|<%/.test(savedText)) throw new Error(`template property or variable leaked into the copy:\n${savedText}`);
  for (const [name, text] of [["Usual note", usual], ["Bulleted note", bulleted]])
    if (fs.readFileSync(path.join(GRAPH, `pages/${name}.md`), "utf8") !== text) throw new Error(`${name} source changed`);
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify({ testedBase, app: APP, savedText, outcome: "PASS: page-header template listed by /template and /usual, inserted with template:: stripped and <% today %> expanded, sources unchanged" }));
  console.log(`PASS GitHub discussion 729: ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(TMP, "failure.txt"), `Tested: ${testedBase}\nJourney: slash menu on a routed page, page-header template. Stage: ${stage}. Observed: ${error.stack}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
