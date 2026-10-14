// Focused native CSS/editor journey; all fixtures and evidence stay in this lane.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
ensurePrivateSessionBus();
const testedCommit = execFileSync("rtk", ["git", "rev-parse", "HEAD"], { encoding: "utf8" }).trim() + " plus working diff";
if (!process.env.DISPLAY) throw new Error("Run under xvfb-run");
const artifacts = path.resolve("artifacts/desktop-misc");
fs.mkdirSync(artifacts, { recursive: true });
const fixture = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(fixture, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "pages/Probe.md"), "public:: true\n- CSS editor probe\n");
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(graph, "journals", `${day}.md`), "- CSS editor probe\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const graphCss = path.join(graph, "logseq/custom.css");
const exportBase = path.join(fixture, "exports");
fs.mkdirSync(path.join(exportBase, "previous"), { recursive: true });
const xdg = path.join(fixture, "xdg");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, dir, APP_ID), { recursive: true });
fs.writeFileSync(path.join(xdg, "data", APP_ID, "tine-settings.json"), JSON.stringify({ last_graph_path: graph, html_export_directory: path.join(exportBase, "previous") }));
const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const driverPort = Number(process.env.E2E_DRIVER_PORT || 4496);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4497);
const log = fs.openSync(path.join(fixture, "driver.log"), "w");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(xdg, "data"), XDG_CONFIG_HOME: path.join(xdg, "config"), XDG_CACHE_HOME: path.join(xdg, "cache"), GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1" };
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"), ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "desktop CSS editing";
async function command(query, label) {
  await browser.keys(["Control", "k"]);
  await browser.keys("NULL");
  const input = await browser.$('.switcher-input');
  await input.waitForDisplayed({ timeout: 10000 });
  await input.setValue(query);
  const option = await browser.$(`//*[@role='option'][contains(., '${label}')]`);
  await option.waitForDisplayed({ timeout: 10000 });
  await option.click();
}
async function click(label) {
  const button = await browser.$(`//button[normalize-space(.)='${label}']`);
  await button.waitForDisplayed({ timeout: 10000 });
  await button.waitForEnabled({ timeout: 10000 });
  await button.click();
  if (label === "Save and close") {
    await browser.$('[role="dialog"][aria-label="CSS editor"]').waitForExist({ timeout: 10000, reverse: true });
  }
}
async function styleSaved(filename, text) {
  await browser.waitUntil(() => fs.existsSync(filename) && fs.readFileSync(filename, "utf8") === text, { timeout: 10000, timeoutMsg: `CSS not persisted: ${filename}` });
}
try {
  const deadline = Date.now() + 10000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${driverPort}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("driver startup failed");
    await delay(150);
  }
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await browser.$(".ls-block").waitForExist({ timeout: 30000 });
  await command("css", "Edit global CSS");
  const globalText = ":root { --tine-bullet-color: rgb(7, 8, 9); }";
  await browser.$('textarea[aria-label="CSS editor"]').setValue(globalText);
  await click("Save");
  await styleSaved(path.join(xdg, "config", APP_ID, "global.css"), globalText);
  await click("Save and close");
  await command("custom.css", "Edit custom.css");
  const graphText = ":root { --tine-bullet-color: rgb(30, 40, 50); }";
  await browser.$('textarea[aria-label="CSS editor"]').setValue(graphText);
  await click("Save");
  await styleSaved(graphCss, graphText);
  await browser.waitUntil(() => browser.execute(() => {
    const global = document.querySelector("#tine-global-css");
    const graph = document.querySelector("#tine-custom-css");
    return global?.textContent.includes("7, 8, 9") && graph?.textContent.includes("30, 40, 50") && Boolean(global.compareDocumentPosition(graph) & Node.DOCUMENT_POSITION_FOLLOWING);
  }), { timeout: 10000, timeoutMsg: "global-before-graph cascade did not apply" });
  try { await browser.saveScreenshot(path.join(fixture, "css-editor-saved.png")); } catch {}
  await click("Save and close");
  await command("safe mode", "Toggle custom CSS");
  assert.equal(await browser.execute(() => document.querySelector("#tine-global-css")?.textContent + document.querySelector("#tine-custom-css")?.textContent), "");
  await command("safe mode", "Toggle custom CSS");
  journey = "background CSS persistence";
  await command("custom.css", "Edit custom.css");
  const backgroundText = ":root { --tine-bullet-color: rgb(60, 70, 80); }";
  await browser.$('textarea[aria-label="CSS editor"]').setValue(backgroundText);
  await browser.execute(() => window.dispatchEvent(new Event("pagehide")));
  await styleSaved(graphCss, backgroundText);
  await click("Save and close");
  journey = "desktop CSS editing";
  if (process.env.E2E_PROBE_EXPORT === "1") {
    journey = "native HTML export";
    await command("Export graph", "Export graph as HTML and read-only app");
    await click("Export HTML and read-only app…");
    const native = (...args) => execFileSync("rtk", ["proxy", "xdotool", ...args], { env, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
    let chooser;
    const chooserDeadline = Date.now() + 15000;
    while (!chooser && Date.now() < chooserDeadline) {
      try { chooser = native("search", "--onlyvisible", "--name", "Choose a folder outside").split("\n")[0]; } catch {}
      if (!chooser) {
        const status = await browser.execute(() => [...document.querySelectorAll('[role="status"]')].find(el => el.textContent?.startsWith("Export failed"))?.textContent ?? "");
        if (status.includes("Export failed")) throw new Error(`native export picker unavailable: ${status}`);
        await delay(150);
      }
    }
    if (!chooser) throw new Error("native export folder chooser did not appear in the headless session");
    native("windowfocus", "--sync", chooser);
    native("key", "Return");
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll('[role="status"]')].some(el => el.textContent?.startsWith("Exported 1 pages"))), { timeout: 30000, timeoutMsg: "HTML export did not report its one public page" });
    const chosen = path.join(exportBase, "previous");
    await browser.waitUntil(() => JSON.parse(fs.readFileSync(path.join(xdg, "data", APP_ID, "tine-settings.json"), "utf8")).html_export_directory === chosen, { timeout: 10000 });
    const output = await browser.execute(() => [...document.querySelectorAll('[role="status"]')].find(el => el.textContent?.startsWith("Exported 1 pages to ")).textContent.slice("Exported 1 pages to ".length));
    assert.ok(output === chosen || output.startsWith(chosen + path.sep), "the result must name the chosen destination");
    assert.ok(fs.existsSync(path.join(output, "index.html")), "the result must point to the generated site");
    await click("Show in folder");
    console.log("PASS: native HTML destination selection, persisted last choice, exported path and reveal action");
    await browser.keys("Escape");
    journey = "desktop CSS editing";
  }
  await command("custom.css", "Edit custom.css");
  const mine = ":root { --tine-bullet-color: red; }";
  await browser.$('textarea[aria-label="CSS editor"]').setValue(mine);
  fs.writeFileSync(graphCss, ":root { --tine-bullet-color: green; }\n");
  await click("Save");
  await browser.$('[role="alert"]').waitForExist({ timeout: 10000 });
  assert.equal(await browser.$('textarea[aria-label="CSS editor"]').getValue(), mine);
  assert.ok(fs.readFileSync(graphCss, "utf8").includes("green"));
  try { await browser.saveScreenshot(path.join(fixture, "css-editor-conflict.png")); } catch {}
  journey = "CSS draft recovery after WebView reload";
  await browser.refresh();
  await browser.$(".ls-block").waitForExist({ timeout: 30000 });
  await command("custom.css", "Edit custom.css");
  assert.equal(await browser.$('textarea[aria-label="CSS editor"]').getValue(), mine);
  await click("Save");
  await browser.$('[role="alert"]').waitForExist({ timeout: 10000 });
  assert.ok(fs.readFileSync(graphCss, "utf8").includes("green"), "recovered draft must retain its original revision");
  console.log("PASS: background pagehide persists CSS and WebView reload recovers a conflicting draft without rebasing");
  console.log("PASS: native CSS quick-switcher entry, guarded graph/global saves, cascade, safe mode, and external conflict retention");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(fixture, "failure.png")); } catch {}
  fs.writeFileSync(path.join(fixture, "failure.json"), JSON.stringify({ testedCommit, journey, expected: journey === "native HTML export" ? "native destination acceptance, exported site, remembered folder and reveal action" : "in-app guarded CSS saves and retained conflicting buffer", observed: String(error), classification: "ambiguous", fixture }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
