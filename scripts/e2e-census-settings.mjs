// Native command → existing workspace form → durable workspace registry.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const run = path.resolve(`artifacts/sw-census-settings/native/run-${Date.now()}`);
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "pages/Census.md"), "- Workspace creation fixture\n");
const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4596);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4597);
const target = await startWebdriverApplication(app, env, nativePort);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env: target.env, stdio: ["ignore", log, log], detached: true });
let browser;
try {
  await sleep(2500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 30_000 });
  for (const [compact, name] of [[false, "Census full"], [true, "Census compact"]]) {
    if (compact) {
      await browser.keys("Escape");
      await browser.keys(["t", "l"]);
      await browser.$('[data-workspace-switcher-compact="true"]').waitForExist({ timeout: 5000 });
    }
    await browser.keys(["Control", "Shift", "p"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5000 });
    await input.setValue("New workspace");
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".switcher-row")].some((row) => row.textContent.includes("New workspace…"))), { timeout: 5000 });
    await browser.keys("Enter");
    const field = await browser.$('.workspace-edit-input[aria-label="New workspace name"]');
    await field.waitForDisplayed({ timeout: 5000 });
    await field.setValue(name);
    await browser.$('.workspace-edit-form button[type="submit"]').click();
    await browser.waitUntil(() => browser.execute((name) => document.querySelector(".workspace-switcher-btn")?.getAttribute("aria-label")?.includes(name)
      || document.querySelector(".workspace-switcher-name")?.textContent?.includes(name), name), { timeout: 10_000 });
    console.log(`PASS: command creates ${name} using ${compact ? "compact" : "sidebar"} switcher`);
  }
  const registries = fs.readdirSync(path.join(run, "data"), { recursive: true }).filter((file) => String(file).endsWith("-workspaces.json"));
  const registry = registries.map((file) => JSON.parse(fs.readFileSync(path.join(run, "data", file), "utf8"))).find((data) =>
    ["Census full", "Census compact"].every((name) => data.workspaces.some((workspace) => workspace.name === name)));
  if (!registry) throw new Error("Created workspaces missing from durable registry");
  console.log("PASS: both created workspaces persisted through the existing guarded registry writer");
} catch (error) {
  await browser?.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ journey: "palette workspace creation", expected: "both switchers create durable named workspaces", observed: String(error), classification: "ambiguous", commit: process.env.TESTED_COMMIT || "working tree" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  stopWebdriverApplication(target); fs.closeSync(log);
}
