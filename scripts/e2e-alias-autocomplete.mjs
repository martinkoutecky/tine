// GH #675 / #676: observe real backend candidates, displayed choices and insertion.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testedCommit = execFileSync("rtk", ["git", "rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const binarySha256 = createHash("sha256").update(fs.readFileSync(APP)).digest("hex");
const TMP = path.join(ROOT, "artifacts/gh675", String(Date.now()));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages/Tine.md"), "alias:: TineOutline, [[Tine Outliner]], SSE\n- body\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), "- Complete a reference here\n");
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg/data"), XDG_CONFIG_HOME: path.join(TMP, "xdg/config"), XDG_CACHE_HOME: path.join(TMP, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4675);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
await sleep(2500);
let browser;
const observations = [];
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP } } });
  await browser.$(".ls-block .block-content-wrapper").waitForExist({ timeout: 20000 });
  await browser.$(".ls-block .block-content-wrapper").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  for (const [typed, choices, selected, inserted] of [
    ["[[tin", ["Tine", "TineOutline", "Tine Outliner"], "TineOutline", "[[TineOutline]]"],
    ["[[sse", ["SSE"], "SSE", "[[SSE]]"],
    ["#tin", ["#Tine", "#TineOutline", "#Tine Outliner"], "#TineOutline", "#TineOutline"],
  ]) {
    await browser.execute((text) => {
      const editor = document.querySelector("textarea.block-editor");
      editor.focus(); editor.value = text; editor.setSelectionRange(text.length, text.length);
      editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text.at(-1) }));
    }, typed);
    const labels = () => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")].map((el) => el.textContent.trim()));
    await browser.waitUntil(async () => {
      const current = await labels();
      return choices.every((name) => current.includes(name));
    }, { timeout: 10000 });
    const actual = (await labels()).filter((name) => !name.startsWith("Create"));
    assert.deepEqual(actual, choices);
    await browser.execute((name) => {
      const row = [...document.querySelectorAll(".autocomplete .ac-label")].find((el) => el.textContent.trim() === name).parentElement;
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    }, selected);
    await browser.waitUntil(async () => (await browser.$("textarea.block-editor").getValue()).trim() === inserted, { timeout: 5000 });
    observations.push({ typed, choices: actual, inserted });
  }
  await browser.saveScreenshot(path.join(TMP, "completion.png"));
  fs.writeFileSync(path.join(TMP, "receipt.json"), JSON.stringify({ testedCommit, binarySha256, candidate: "uncommitted gh675 diff", outcome: "pass", observations }, null, 2));
  console.log(`PASS: alias casing, owner siblings, page/tag insertion; ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
  fs.writeFileSync(path.join(TMP, "receipt.json"), JSON.stringify({ testedCommit, binarySha256, candidate: "uncommitted gh675 diff", outcome: "fail", journey: "alias completion", expected: "authored alias options and insertion", observed: String(error), observations, classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
