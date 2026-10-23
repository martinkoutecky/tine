// GH #363/#705: native pointer journey through preview navigation and its menu.
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const base = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const run = path.join(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/sw-preview-tabs/native"), `run-${Date.now()}`);
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const uuid = "57000000-0000-4000-8000-000000000001";
const source = `- Source block\n  id:: ${uuid}\n  - Preview child\n- Sibling context\n`;
const orgUuid = "57000000-0000-4000-8000-000000000002";
const orgSource = `* Org source block\n:PROPERTIES:\n:id: ${orgUuid}\n:END:\n** Preview child\n* Sibling context\n`;
fs.writeFileSync(path.join(graph, "pages/Preview Source.md"), source);
fs.writeFileSync(path.join(graph, "pages/Preview Org.org"), orgSource);
const editSource = "- Edit me\n- Second row\n";
fs.writeFileSync(path.join(graph, "pages/Preview Edit.md"), editSource);
fs.writeFileSync(path.join(graph, "pages/Preview Home.md"), `- [[Preview Source]]\n- ((${uuid}))\n- [[Preview Org]]\n- ((${orgUuid}))\n- [[Preview Edit]]\n`);
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4792);
const nativePort = port + 1;
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const target = await startWebdriverApplication(app, env, nativePort);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: target.env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  // GH #748: both popup entrances fold the actual file, including an unopened
  // source, through ordinary document undo. No editor or test-only mutation.
  for (const [name, id, file, original, property] of [
    ["Preview Source", uuid, "pages/Preview Source.md", source, "collapsed:: true"],
    ["Preview Org", orgUuid, "pages/Preview Org.org", orgSource, ":collapsed: true"],
  ]) {
    for (const anchor of [`//a[contains(@class,'page-ref') and contains(.,'${name}')]`, `.page-section .block-ref[data-block-ref="${id}"]`]) {
      stage = `${file} ${anchor} preview fold persists and undo restores`;
      await openPageByName(browser, "Preview Home");
      await browser.$(anchor).moveTo();
      const previewRow = await browser.$(".peek-popup .block-content");
      await previewRow.waitForDisplayed({ timeout: 10000 });
      await previewRow.moveTo(); // disclosure arrows appear on row hover
      const arrow = await browser.$(".peek-popup .collapse-toggle.has-children");
      await arrow.waitForDisplayed({ timeout: 10000 });
      await arrow.click();
      await waitForFileText(path.join(graph, file), text => text.toLowerCase().includes(property));
      await browser.waitUntil(() => browser.execute(() => {
        const popup = document.querySelector(".peek-popup");
        return !!popup && !popup.textContent.includes("Preview child");
      }), { timeout: 10000, timeoutMsg: stage });
      await browser.keys(["Control", "z"]);
      await waitForFileText(path.join(graph, file), text => text === original);
      await browser.waitUntil(() => browser.execute(() => document.querySelector(".peek-popup")?.textContent.includes("Preview child")), { timeout: 10000 });
      await browser.$(".page-title").moveTo();
      await browser.waitUntil(async () => !(await browser.$(".peek-popup").isExisting()), { timeout: 5000 });
      console.log(`PASS: ${stage}`);
    }
  }
  for (const anchor of [".page-ref", ".block-ref"]) {
    for (const action of [".peek-source-title", ".peek-open-source", ".bullet-container"]) {
      stage = `${anchor} preview ${action} activates source`;
      await openPageByName(browser, "Preview Home");
      await browser.$(`.page-section ${anchor}`).moveTo();
      await browser.$(`.peek-popup ${action}`).waitForDisplayed({ timeout: 10000 });
      const before = await browser.execute(() => document.querySelector(".tab.active")?.getAttribute("data-tab-id"));
      await browser.$(`.peek-popup ${action}`).click();
      await browser.waitUntil(() => browser.execute((before) => {
        const active = document.querySelector(".tab.active");
        return active?.getAttribute("data-tab-id") !== before && active?.textContent?.includes("Preview Source");
      }, before), { timeout: 10000, timeoutMsg: stage });
      if (action === ".bullet-container" || (anchor === ".block-ref" && action === ".peek-open-source")) {
        await browser.$(".zoomed-page").waitForExist({ timeout: 10000 });
      }
      console.log(`PASS: ${stage}`);
    }
    stage = `${anchor} preview survives its portaled menu`;
    await openPageByName(browser, "Preview Home");
    await browser.$(`.page-section ${anchor}`).moveTo();
    await browser.$(".peek-page-actions").waitForDisplayed({ timeout: 10000 });
    await browser.$(".peek-page-actions").click();
    const menuItem = await browser.$('[data-page-action-id="open-new-tab"]');
    await menuItem.waitForDisplayed({ timeout: 10000 });
    await menuItem.moveTo();
    // Go beyond the hover grace period to observe ownership across the portal.
    await sleep(400);
    if (!(await browser.$(".peek-popup").isDisplayed())) throw new Error("preview disappeared while its menu was open");
    fs.writeFileSync(path.join(run, `${anchor.slice(1)}-menu.json`), JSON.stringify(await browser.execute(() => ({
      preview: document.querySelector(".peek-popup")?.textContent,
      menu: document.querySelector(".ctx-menu")?.textContent,
      activeTab: document.querySelector(".tab.active")?.textContent,
    })), null, 2));
    console.log(`PASS: ${stage}`);
    const before = await browser.execute(() => ({ active: document.querySelector(".tab.active")?.getAttribute("data-tab-id"), count: document.querySelectorAll(".tab").length }));
    stage = `${anchor} menu confirms background tab`;
    await menuItem.click();
    await browser.waitUntil(() => browser.execute((before) => document.querySelectorAll(".tab").length === before.count + 1
      && document.querySelector(".tab.active")?.getAttribute("data-tab-id") === before.active
      && document.body.textContent.includes("Opened in a new tab"), before), { timeout: 10000 });
    await browser.$(".page-title").moveTo();
    await browser.waitUntil(async () => !(await browser.$(".peek-popup").isExisting()), { timeout: 5000 });
    console.log(`PASS: ${stage} and resumes dismissal`);
  }
  if (fs.readFileSync(path.join(graph, "pages/Preview Source.md"), "utf8") !== source) throw new Error("preview navigation wrote source content");
  if (fs.readFileSync(path.join(graph, "pages/Preview Org.org"), "utf8") !== orgSource) throw new Error("preview fold undo changed Org content");
  // D-20261023-VQ: the preview's blocks are live. A click edits in place, the
  // popup holds while its editor has the caret, the first Escape ends the edit.
  stage = "preview edits its live source block in place";
  await openPageByName(browser, "Preview Home");
  await browser.$("//a[contains(@class,'page-ref') and contains(.,'Preview Edit')]").moveTo();
  const editRow = await browser.$("//div[contains(@class,'peek-popup')]//div[contains(@class,'block-content') and contains(.,'Edit me')]");
  await editRow.waitForDisplayed({ timeout: 10000 });
  await editRow.click();
  const previewEditor = await browser.$(".peek-popup textarea.block-editor");
  await previewEditor.waitForDisplayed({ timeout: 10000, timeoutMsg: `${stage}: no editor in the preview` });
  await browser.keys(["End"]);
  await browser.keys(" now");
  await browser.$(".page-title").moveTo();
  await sleep(1000); // well past the hover grace period
  if (!(await browser.$(".peek-popup").isDisplayed())) throw new Error("preview closed while its editor held the caret");
  await browser.keys(["Escape"]);
  await browser.waitUntil(async () => !(await browser.$(".peek-popup textarea.block-editor").isExisting()), { timeout: 5000, timeoutMsg: `${stage}: Escape did not end the edit` });
  if (!(await browser.$(".peek-popup").isExisting())) throw new Error("the first Escape closed the preview instead of ending the edit");
  await waitForFileText(path.join(graph, "pages/Preview Edit.md"), text => text === "- Edit me now\n- Second row\n");
  await browser.keys(["Escape"]);
  await browser.waitUntil(async () => !(await browser.$(".peek-popup").isExisting()), { timeout: 5000, timeoutMsg: `${stage}: second Escape kept the preview` });
  console.log(`PASS: ${stage}`);
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ outcome: "pass", testedBase: base, delta: "sw-preview-tabs worktree diff", graph }, null, 2));
  console.log("PASS: page and block preview folds persist in Markdown/Org and undo; source tabs, menus and dismissal work; previews edit in place");
} catch (error) {
  await browser?.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ testedBase: base, journey: stage,
    expected: "preview fold persists as collapsed in Markdown/Org; undo restores bytes; foreground source navigation; preview retained with menu; background tab confirmation", observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  stopWebdriverApplication(target);
  fs.closeSync(log);
}
