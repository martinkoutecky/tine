// GH #711: native navigation must activate the destination tab and preserve
// the source tab, including logical-name versus physical-file entry paths.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/sw-tabreuse/native");
const RUN = path.join(ARTIFACT, `run-${Date.now()}`);
const GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(RUN, dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), '{:favorites ["Reuse Alpha"]}\n');
fs.writeFileSync(path.join(GRAPH, "pages/Reuse Alpha.md"), "- Alpha destination content\n");
fs.writeFileSync(path.join(GRAPH, "pages/Reuse Beta.md"), "- Beta source content [[Reuse Alpha]]\n");
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"), XDG_CACHE_HOME: path.join(RUN, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4582);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4583);
const target = await startWebdriverApplication(APP, env, nativePort);
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const td = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), {
  env: target.env, stdio: ["ignore", log, log], detached: true,
});
let browser;
try {
  await sleep(2500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  console.log("Ready: isolated native graph");

  async function search(name, background = false) {
    console.log(`Search: ${name}, background=${background}`);
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5000 });
    await input.setValue(name);
    await browser.waitUntil(() => browser.execute((name) => [...document.querySelectorAll(".switcher-row")].some((row) =>
      row.querySelector(".switcher-kind")?.textContent?.trim() === "page" && row.querySelector(".switcher-name")?.textContent?.trim() === name), name), { timeout: 10_000 });
    if (background) {
      await browser.execute((name) => {
        const row = [...document.querySelectorAll(".switcher-row")].find((row) => row.querySelector(".switcher-kind")?.textContent?.trim() === "page"
          && row.querySelector(".switcher-name")?.textContent?.trim() === name);
        row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 1 }));
      }, name);
      await browser.keys("Escape");
    } else await browser.keys("Enter");
  }
  async function activate(name) {
    await browser.execute((name) => [...document.querySelectorAll(".tab")].find((tab) => tab.querySelector(".tab-title")?.textContent?.trim() === name)?.click(), name);
    await browser.waitUntil(() => browser.execute((name) => document.querySelector(".page-title")?.textContent?.includes(name), name), { timeout: 10_000 });
  }
  async function assertReused(method) {
    console.log(`Verify: ${method}`);
    await browser.waitUntil(() => browser.execute(() => document.querySelector(".tab.active .tab-title")?.textContent?.trim() === "Reuse Alpha"), {
      timeout: 5000, timeoutMsg: `${method}: destination tab was not activated`,
    });
    const titles = await browser.execute(() => [...document.querySelectorAll(".tab .tab-title")].map((el) => el.textContent.trim()));
    if (titles.filter((name) => name === "Reuse Alpha").length !== 1 || !titles.includes("Reuse Beta")) throw new Error(`${method}: source tab changed: ${JSON.stringify(titles)}`);
    await activate("Reuse Beta");
    if (!(await browser.$(".page-blocks").getText()).includes("Beta source content")) throw new Error(`${method}: source content lost`);
    console.log(`PASS: ${method}`);
  }
  await search("Reuse Alpha", true);
  await search("Reuse Beta");
  await browser.$(".page-ref[data-page='Reuse Alpha'], .page-ref").waitForExist({ timeout: 10_000 });
  await browser.$(".page-ref[data-page='Reuse Alpha'], .page-ref").click();
  await assertReused("internal link");
  await search("Reuse Alpha");
  await assertReused("Ctrl+K Enter");
  // Recent records physical ownership; Favorites use logical names. Both are
  // production sidebar entry paths rather than calls into the router.
  await browser.execute(() => {
    const link = [...document.querySelectorAll("#sidebar-recent-list .nav-page")].find((el) => el.textContent.trim() === "Reuse Alpha");
    if (!link) throw new Error("Recent page link missing");
    link.click();
  });
  await assertReused("Recent sidebar");
  await browser.execute(() => {
    const link = [...document.querySelectorAll("#sidebar-favorites-list .nav-page")].find((el) => el.textContent.trim() === "Reuse Alpha");
    if (!link) throw new Error("Favorite page link missing");
    link.click();
  });
  await assertReused("Favorites sidebar");
  console.log("PASS: link, Ctrl+K Enter, Recent and Favorites activate Alpha and preserve Beta");
} catch (error) {
  if (browser) {
    await browser.saveScreenshot(path.join(RUN, "failure.png")).catch(() => {});
    const state = await browser.execute(() => ({ tabs: [...document.querySelectorAll(".tab")].map((el) => ({ title: el.textContent, active: el.classList.contains("active") })), title: document.querySelector(".page-title")?.textContent })).catch(() => null);
    fs.writeFileSync(path.join(RUN, "failure.json"), JSON.stringify({ error: String(error), state }, null, 2));
  }
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(target);
  fs.closeSync(log);
}
