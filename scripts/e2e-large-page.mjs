// GH #623: actual native 2,000-block page, far-down edit, navigation and history.
// Run under xvfb-run -s "-screen 0 1920x1080x24" with TINE_APP set.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { setTimeout as delay } from "node:timers/promises";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "tine-large-page-"));
const graph = `${tmp}/graph`;
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${graph}/${dir}`, { recursive: true });
const file = `${graph}/pages/Large page.md`;
const original = Array.from({ length: 2000 }, (_, i) => `- Row ${i} native payload\n`).join("");
fs.writeFileSync(file, original);
fs.writeFileSync(`${graph}/pages/Away.md`, "- A small destination\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${tmp}/xdg/${dir}`, { recursive: true });
const port = Number(process.env.E2E_DRIVER_PORT || 4540);
const log = fs.openSync(`${tmp}/driver.log`, "w");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: `${tmp}/xdg/data`, XDG_CONFIG_HOME: `${tmp}/xdg/config`, XDG_CACHE_HOME: `${tmp}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const td = spawn(process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt >= 100) throw new Error("Driver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.resolve("target/debug/tine") } } });
  await browser.$(".page-title").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Large page");
  console.log("opened large page");
  const shells = await browser.$$(".ls-block");
  if (shells.length >= 300) throw new Error(`Large page mounted ${shells.length} shells`);
  await browser.keys(["Control", "f"]);
  const find = await browser.$(".inpage-find-input");
  await find.waitForExist(); await find.setValue("Row 1800 native");
  await browser.waitUntil(() => browser.execute(() => {
    const row = [...document.querySelectorAll(".block-content")].find((e) => e.textContent.includes("Row 1800 native"));
    const sc = document.querySelector(".main-content");
    if (!row || !sc) return false;
    const r = row.getBoundingClientRect(), v = sc.getBoundingClientRect();
    return r.top >= v.top && r.bottom <= v.bottom;
  }), { timeout: 15_000, timeoutMsg: "Find did not render and reveal the far-down row" });
  console.log("Find revealed row 1800");
  await browser.keys("Escape");
  const id = await browser.execute(() => [...document.querySelectorAll(".block-content")]
    .find((e) => e.textContent.includes("Row 1800 native")).closest(".ls-block").dataset.blockId);
  await browser.$(`[data-block-id="${id}"] .block-content`).click();
  const editor = await browser.$("textarea.block-editor");
  await editor.waitForExist();
  console.log("editor focused");
  await browser.keys(["Control", "a"]);
  await browser.keys("Far row edited and saved");
  await browser.keys("Escape");
  await waitForFileText(file, (text) => text.includes("- Far row edited and saved\n"));
  console.log("far edit saved");
  const offset = await browser.execute(() => document.querySelector(".main-content").scrollTop);
  if (offset < 20_000) throw new Error("The edited block was not far down the page");
  await openPageByName(browser, "Away");
  await browser.$('button[title="Go back"]').click();
  await browser.waitUntil(() => browser.execute((target) => {
    const title = document.querySelector(".page-title")?.textContent.trim();
    const sc = document.querySelector(".main-content");
    const row = [...document.querySelectorAll(".block-content")].find((e) => e.textContent.includes("Far row edited and saved"));
    if (title !== "Large page" || !row || !sc) return false;
    const r = row.getBoundingClientRect(), v = sc.getBoundingClientRect();
    return Math.abs(sc.scrollTop - target) < 100 && r.bottom > v.top && r.top < v.bottom;
  }, offset), { timeout: 15_000, timeoutMsg: "Back did not restore the edited row and saved scroll" });
  const saved = fs.readFileSync(file, "utf8");
  if (saved !== original.replace("- Row 1800 native payload\n", "- Far row edited and saved\n")) {
    throw new Error("Far edit changed other blocks or file bytes");
  }
  await browser.saveScreenshot(`${tmp}/back.png`);
  console.log(`PASS: far-down edit saved; Back restored the row and scroll. Artifacts: ${tmp}`);
} catch (error) {
  if (browser) {
    try { await browser.saveScreenshot(`${tmp}/failure.png`); fs.writeFileSync(`${tmp}/failure.html`, await browser.getPageSource()); } catch {}
  }
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
