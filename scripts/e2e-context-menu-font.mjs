// GH #916: compare real computed typography across routed page/block menus.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "target/artifacts/fix-beta-menu-font-916"));
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-"));
const GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(RUN, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
fs.writeFileSync(path.join(GRAPH, "pages/Menu fonts.md"), "- Parent menu fixture\n  - Child menu fixture\n");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const revision = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const fixture = accountHomeFixture(RUN, RUN);
const env = { ...fixture.env, HOME: path.join(RUN, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"),
  XDG_CACHE_HOME: path.join(RUN, "cache"), XDG_STATE_HOME: path.join(RUN, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4916);
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
const screenshot = name => execFileSync("import", ["-window", "root", path.join(RUN, `${name}.png`)], { timeout: 10000 });
const measurements = [];
let browser, stage = "startup", classification = "harness/infrastructure";
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  await openPageByName(browser, "Menu fonts");
  const faults = [];
  for (const [layout, width] of [["desktop", 1400], ["narrow-touch-styles", 600]]) {
    await browser.setWindowSize(width, 1000);
    await browser.execute(touch => document.documentElement.toggleAttribute("data-touch-gestures", touch), width === 600);
    for (const kind of ["page", "block"]) {
      stage = `${layout} ${kind} menu and submenu`;
      const target = await browser.$(kind === "page" ? "h1.page-title" : ".main-content .ls-block .bullet");
      await target.click({ button: "right" });
      await browser.$(".ctx-menu").waitForExist({ timeout: 5000 });
      // Block copy actions are top-level (OG parity); exercise its view submenu.
      const submenu = await browser.$(kind === "page" ? '[data-page-action-id="copy-as"]' : ".ctx-submenu");
      await submenu.moveTo();
      await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".ctx-submenu:hover > .ctx-submenu-menu")
        && getComputedStyle(document.querySelector(".ctx-submenu:hover > .ctx-submenu-menu")).visibility === "visible"),
      { timeout: 5000, timeoutMsg: "hover did not reveal the submenu" });
      const measured = await browser.execute(() => {
        const rows = [...document.querySelectorAll(".ctx-menu .ctx-item")].filter(row => {
          const style = getComputedStyle(row);
          return style.visibility === "visible" && row.getClientRects().length > 0;
        });
        return { bodyFamily: getComputedStyle(document.body).fontFamily,
          rows: rows.map(row => ({ label: row.firstChild?.textContent?.trim(),
            submenu: !!row.closest(".ctx-submenu-menu"),
            size: getComputedStyle(row).fontSize, family: getComputedStyle(row).fontFamily })) };
      });
      measurements.push({ layout, kind, ...measured });
      await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(() => done())));
      screenshot(`${layout}-${kind}`);
      assert.ok(measured.rows.some(row => row.submenu), "submenu rows were not measured");
      assert.ok(measured.rows.some(row => !row.submenu), "parent menu rows were not measured");
      for (const row of measured.rows) {
        if (row.size !== measured.rows[0].size || row.family !== measured.bodyFamily) faults.push({ layout, kind, ...row });
      }
      await browser.keys("Escape");
      await browser.$(".ctx-menu").waitForExist({ reverse: true, timeout: 5000 });
    }
  }
  stage = "uniform typography across every menu";
  const size = measurements[0].rows[0].size;
  for (const menu of measurements) {
    if (menu.rows.some(row => row.size !== size)) faults.push({ layout: menu.layout, kind: menu.kind, expectedSize: size });
  }
  fs.writeFileSync(path.join(RUN, "fonts.json"), JSON.stringify(measurements, null, 2));
  classification = "product";
  assert.deepEqual(faults, [], "every visible context-menu row and submenu row must share size and interface family");
  fixture.verify();
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ outcome: "PASS", revision, app: APP, measurements }, null, 2));
  console.log(`PASS ${RUN}`);
} catch (error) {
  try { screenshot("failure"); } catch {}
  fs.writeFileSync(path.join(RUN, "failure.json"), JSON.stringify({ revision, app: APP, journey: stage,
    expected: "Uniform font size and interface family across routed page/block menus and hovered submenus",
    observed: error.stack, classification, measurements }, null, 2));
  console.error(`FAIL ${RUN}`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close?.();
}
