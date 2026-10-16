// Built native Diagnostics retains its real reporting backend after parser-runner removal.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as delay } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const out = path.resolve(`artifacts/sw-r2-polish2/native/run-${Date.now()}`);
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "pages/Polish.md"), "- Native diagnostics fixture\n");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4594);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4595);
const target = await startWebdriverApplication(app, env, nativePort);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), {
  env: target.env, stdio: ["ignore", log, log], detached: true,
});
let browser;
try {
  await delay(2500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  console.log("Ready: isolated native app window");
  await browser.setWindowSize(1280, 900);
  await browser.$('button.icon-btn[title^="Settings"]').waitForExist({ timeout: 30_000 });
  await browser.$('button.icon-btn[title^="Settings"]').click();
  await browser.$(".settings-search-input").waitForExist();
  console.log("Ready: Settings");
  for (const locale of ["en", "zh-CN"]) {
    await browser.execute(locale => {
      const select = [...document.querySelectorAll("select")].find(el => el.querySelector('option[value="zh-CN"]'));
      select.value = locale;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    }, locale);
    await browser.waitUntil(() => browser.execute(locale => document.documentElement.lang === locale, locale));
    await browser.$(".settings-search-input").setValue(locale === "en" ? "sticky" : "父");
    await browser.$(".settings-search-result mark").waitForExist();
    assert.equal((await browser.$(".settings-search-result mark").getText()).toLowerCase(), locale === "en" ? "sticky" : "父");
    await browser.saveScreenshot(path.join(out, `settings-${locale}.png`));
    console.log(`PASS: highlighted Settings result (${locale})`);
    await browser.execute(() => {
      const search = document.querySelector(".settings-search-input");
      search.value = "";
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    });
  }
  await browser.execute(() => {
    const select = [...document.querySelectorAll("select")].find(el => el.querySelector('option[value="zh-CN"]'));
    select.value = "en";
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await browser.waitUntil(() => browser.execute(() => document.documentElement.lang === "en"));
  await browser.execute(() => [...document.querySelectorAll(".settings-nav-item")].find(el => el.textContent.includes("Help & diagnostics")).click());
  await browser.$(".diagnostics-tab").waitForExist();
  const diagnostics = await browser.$(".diagnostics-tab").getText();
  assert.ok(!diagnostics.includes("Help improve Tine's parser"));
  for (const label of ["Create diagnostic report", "Save report", "Clear recorded events", "Rescan graph", "Verify synchronized graph"]) assert.ok(diagnostics.includes(label), label);
  await browser.execute(() => [...document.querySelectorAll(".diagnostics-actions button")].find(el => el.textContent === "Create diagnostic report").click());
  await browser.$(".diagnostics-preview textarea").waitForExist({ timeout: 15_000 });
  assert.ok((await browser.$(".diagnostics-preview textarea").getValue()).length > 0);
  await browser.saveScreenshot(path.join(out, "diagnostics-report.png"));
  await browser.execute(() => [...document.querySelectorAll(".diagnostics-actions button")].find(el => el.textContent === "Clear recorded events").click());
  await browser.waitUntil(async () => !(await browser.$(".diagnostics-preview").isExisting()));
  await browser.$(".diagnostics-rescan button").click();
  await browser.waitUntil(async () => (await browser.$(".diagnostics-rescan").getText()).includes("Last rescan finished"), { timeout: 20_000 });
  console.log("PASS: native English/Chinese Settings highlights; Diagnostics controls, real report, clear and rescan");
} catch (error) {
  await browser?.saveScreenshot(path.join(out, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ commit: "2b1c606c1916b4135e764fc3b04abce575840316 + working diff", journey: "native settings highlights and Diagnostics", expected: "highlighted results and report/clear/rescan without parser panel", observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  stopWebdriverApplication(target);
  fs.closeSync(log);
}
