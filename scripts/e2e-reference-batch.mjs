// R2 reference lane: native source identity, marker references and Favorites.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/references/native"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const uuid = "57000000-0000-4000-8000-000000000001";
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:favorites [] :tine/favorites-page "Favorites"}');
fs.writeFileSync(path.join(graph, "pages/Favorites.md"), `tine/favorites:: true\n\n- ((${uuid}))\n`);
fs.writeFileSync(path.join(graph, "pages/Canonical.md"), `alias:: Nickname\nstatus:: active\n\n- Important block\n  id:: ${uuid}\n- TODO call Alice\n- Bookmark this block\n`);
fs.writeFileSync(path.join(graph, "pages/Home.md"), "- [[Nickname]]\n- [[TODO]]\n");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const driverPath = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4726);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(driverPath, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Home");
  stage = "alias source preview";
  const alias = await browser.$('a.page-ref*=Nickname');
  await alias.moveTo();
  await browser.$(".peek-popup").waitForDisplayed({ timeout: 10_000 });
  const preview = await browser.$(".peek-popup").getText();
  if (!preview.includes("Canonical") || !preview.includes("status") || !preview.includes("active")) throw new Error(`missing source identity or properties: ${preview}`);
  await browser.$('.peek-popup [aria-label="Open source"]').click();
  await browser.waitUntil(async () => browser.execute(() => [...document.querySelectorAll(".tab")].some(node => node.textContent?.includes("Canonical"))), { timeout: 10_000 });
  stage = "task marker references";
  await openPageByName(browser, "TODO");
  await browser.$(".linked-references").waitForExist({ timeout: 10_000 });
  if (!(await browser.$(".linked-references").getText()).includes("call Alice")) throw new Error("TODO task missing from Linked References");
  const checkbox = await browser.$('.linked-references [role="checkbox"]');
  await checkbox.waitForDisplayed({ timeout: 10_000 });
  await checkbox.click();
  await browser.waitUntil(() => fs.readFileSync(path.join(graph, "pages/Canonical.md"), "utf8").includes("DONE call Alice"), { timeout: 10_000 });
  stage = "block favorite zoom";
  await browser.waitUntil(async () => (await browser.$("#sidebar-favorites-list").getText()).includes("Important block"), { timeout: 10_000 });
  await browser.$("#sidebar-favorites-list .nav-page").click();
  await browser.waitUntil(async () => browser.execute(() => {
    const page = document.querySelector(".zoomed-page");
    return !!page?.textContent?.includes("Important block") && !page?.textContent?.includes("call Alice");
  }), { timeout: 10_000 });
  stage = "Favourite block context action";
  await openPageByName(browser, "Canonical");
  const sourceBlock = await browser.$('//div[contains(@class,"block-content") and normalize-space(.)="Bookmark this block"]');
  await sourceBlock.click({ button: "right" });
  const favoriteAction = await browser.$('//div[contains(@class,"ctx-item") and normalize-space(.)="Favourite block"]');
  await favoriteAction.waitForDisplayed({ timeout: 10_000 });
  await favoriteAction.click();
  await browser.waitUntil(() => {
    const source = fs.readFileSync(path.join(graph, "pages/Canonical.md"), "utf8");
    const favorites = fs.readFileSync(path.join(graph, "pages/Favorites.md"), "utf8");
    return source.includes("Bookmark this block\n  id::") && favorites.split("((").length === 3;
  }, { timeout: 10_000 });
  stage = "Favorites derived star";
  await openPageByName(browser, "Favorites");
  await browser.waitUntil(async () => (await browser.$(".page-section .block-ref").getText()).includes("Important block"), { timeout: 10_000 });
  const favorite = await browser.$(".page-section .block-ref").getText();
  const starred = await browser.execute(() => {
    const ref = document.querySelector(".page-section .block-ref");
    return !!ref?.querySelector('img[alt="⭐"]') || !!ref?.textContent?.includes("⭐");
  });
  if (!starred) throw new Error(`missing derived star: ${favorite}`);
  const config = fs.readFileSync(path.join(graph, "logseq/config.edn"), "utf8");
  if (config.includes(uuid)) throw new Error("block favorite leaked into config.edn");
  await browser.saveScreenshot(path.join(run, "pass.png"));
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ outcome: "pass", testedBase: "88700137bc981033ba78a8896424464b6c1d2fdc + reference lane diff", preview, favorite, graph }, null, 2));
  console.log("PASS: native alias properties, source navigation, tickable task-marker backlinks, block favorite zoom and derived star");
} catch (error) {
  await browser?.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ testedBase: "88700137bc981033ba78a8896424464b6c1d2fdc + reference lane diff", journey: stage, expected: "source preview and navigation; durable task toggle; Favorites block zoom and derived star", observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
