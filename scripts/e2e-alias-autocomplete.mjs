// GH #675 / #676: observe real backend candidates, displayed choices and insertion.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { measureCompletionLatency } from "./lib/e2e-completion-latency.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();
// Install inside the private-bus child so its lease sampler stays responsive.
await import("./lib/e2e-auto-account-home.mjs");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const binarySha256 = createHash("sha256").update(fs.readFileSync(APP)).digest("hex");
const latency = process.env.TINE_LATENCY_GRAPH;
const candidate = process.env.TINE_LATENCY_LABEL || "working tree";
const TMP = latency
  ? path.join(ROOT, "target/artifacts/og-ac-latency-899", `native-${process.env.TINE_LATENCY_LABEL || Date.now()}`)
  : path.join(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/gh675"), String(Date.now()));
const GRAPH = path.join(TMP, "graph");
if (latency) fs.cpSync(latency, GRAPH, { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state", "home"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages/Tine.md"), "alias:: TineOutline, [[Tine Outliner]], SSE\n- body\n");
// GH #888: two pages sharing a prefix; the link being changed names the second one.
fs.writeFileSync(path.join(GRAPH, "pages/sat-01.md"), "title:: Sat, 01.11.2026\n\n- first\n");
fs.writeFileSync(path.join(GRAPH, "pages/sat-07.md"), "title:: Sat, 07.11.2026\n\n- second\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
if (!latency) fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), "- Complete a reference here\n");
const env = { ...process.env, TINE_GRAPH: GRAPH, HOME: path.join(TMP, "xdg/home"), TINE_AC_PROBE: latency ? "1" : "",
  XDG_DATA_HOME: path.join(TMP, "xdg/data"), XDG_CONFIG_HOME: path.join(TMP, "xdg/config"), XDG_CACHE_HOME: path.join(TMP, "xdg/cache"),
  XDG_STATE_HOME: path.join(TMP, "xdg/state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4675);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
const observations = [];
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP } } });
  // Window handles can exist before their documents navigate away from
  // about:blank. Re-select until the main app's navigation is actually there.
  await browser.waitUntil(async () => {
    await ensureMainWindow(browser);
    return browser.$(".app-container").isExisting();
  }, { timeout: 20_000, interval: 100, timeoutMsg: "the main app did not mount" });
  // The window paints before the graph's background name/alias index is
  // warm. Use the same graph-scoped signal as frontend whole-graph readers.
  await browser.waitUntil(async () => {
    const result = await browser.executeAsync(done => {
      window.__TAURI_INTERNALS__.invoke("warm_done").then(
        ready => done({ ready }), error => done({ failure: String(error) }));
    });
    if (result.failure) throw new Error(result.failure);
    return result.ready;
  }, {
    timeout: 20_000, interval: 100, timeoutMsg: "the graph's name/alias index did not finish warming",
  });
  await openPageByName(browser, "Tine");
  await browser.$(".ls-block .block-content-wrapper").waitForExist({ timeout: 20000 });
  await browser.$(".ls-block .block-content-wrapper").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  if (latency) {
    const measurements = await measureCompletionLatency(browser, path.join(GRAPH, "pages/Tine.md"));
    fs.writeFileSync(path.join(TMP, "latency.json"), JSON.stringify({ testedCommit, binarySha256, candidate, measurements }, null, 2));
    console.log(`Latency samples: ${TMP}`);
    await browser.$(".ls-block .block-content-wrapper").click();
    await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  }
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
    assert.deepEqual(latency ? actual.filter(name => choices.includes(name)) : actual, choices);
    await browser.execute((name) => {
      const row = [...document.querySelectorAll(".autocomplete .ac-label")].find((el) => el.textContent.trim() === name).parentElement;
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    }, selected);
    await browser.waitUntil(async () => (await browser.$("textarea.block-editor").getValue()).trim() === inserted, { timeout: 5000 });
    observations.push({ typed, choices: actual, inserted });
  }
  // GH #888 (D-20261109-LK): typing inside a complete link searches the whole
  // link text, so the link's own page is the first choice (Logseq searches
  // only up to the caret); picking it leaves one link.
  if (!latency) {
    await browser.execute(() => {
      const editor = document.querySelector("textarea.block-editor");
      const text = "[[Sat, 07.11.2026]]", caret = "[[Sat,".length;
      editor.focus(); editor.value = text; editor.setSelectionRange(caret, caret);
      editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "," }));
    });
    const labels = () => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")].map((el) => el.textContent.trim()));
    await browser.waitUntil(async () => (await labels()).includes("Sat, 07.11.2026"), { timeout: 10000 });
    const choices = await labels();
    assert.equal(choices[0], "Sat, 07.11.2026");
    await browser.execute(() => {
      const row = document.querySelector(".autocomplete .ac-label").parentElement;
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });
    await browser.waitUntil(async () => (await browser.$("textarea.block-editor").getValue()).trim() === "[[Sat, 07.11.2026]]", { timeout: 5000 });
    observations.push({ typed: "[[Sat,| 07.11.2026]]", choices, inserted: "[[Sat, 07.11.2026]]" });
  }
  await browser.saveScreenshot(path.join(TMP, "completion.png"));
  fs.writeFileSync(path.join(TMP, "receipt.json"), JSON.stringify({ testedCommit, binarySha256, candidate, outcome: "pass", observations }, null, 2));
  console.log(`PASS: alias casing, owner siblings, page/tag insertion; ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
  fs.writeFileSync(path.join(TMP, "receipt.json"), JSON.stringify({ testedCommit, binarySha256, candidate, outcome: "fail", journey: "alias completion", expected: "authored alias options and insertion", observed: String(error), observations, classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
