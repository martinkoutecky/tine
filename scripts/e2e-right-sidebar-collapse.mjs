// Linux real-app proof for GH #106. Exercises semantic keyboard disclosures,
// edit-safe unmounting, collection bulk controls, and native restart restore.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-right-sidebar-collapse-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE_A = `${GRAPH}/pages/Page A.md`;
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

fs.rmSync(TMP, { recursive: true, force: true });
fs.mkdirSync(ARTIFACT, { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE_A, "- Sidebar fold parent\n  id:: sidebar-fold-parent-159\n  - Sidebar fold child\n    id:: sidebar-fold-child-159\n- Editable sidebar text\n- Open [[Page B]]\n");
fs.writeFileSync(`${GRAPH}/pages/Page B.md`, "- Open [[Page A]]\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Page A]] and [[Page B]] in the sidebar\n");

const baseEnv = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};

async function withApp(index, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const log = fs.openSync(`${TMP}/tauri-driver-${index}.log`, "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
    env: baseEnv, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
    await fn(browser);
    await sleep(650);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

async function waitItemCount(browser, count) {
  await browser.waitUntil(() => browser.execute((expected) =>
    document.querySelectorAll(".right-sidebar-body > .rs-item").length === expected, count), {
    timeout: 10_000, timeoutMsg: `right sidebar did not reach ${count} items`,
  });
}

async function itemSelector(name) {
  return `[data-sidebar-surface="sidebar:page:page:${name}"]`;
}

async function openPagesInSidebar(browser) {
  const navigate = async (name) => {
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5000 });
    await input.setValue(name);
    // Ctrl-K initially contains recent rows and updates results asynchronously.
    // Clicking the first row immediately after setValue can therefore select the
    // preceding page while the requested query is still in flight. Wait for and
    // physically click the exact non-block page row the user intended.
    const row = await browser.$(
      `//*[contains(concat(' ', normalize-space(@class), ' '), ' switcher-row ') and not(contains(concat(' ', normalize-space(@class), ' '), ' block-result '))][.//*[contains(concat(' ', normalize-space(@class), ' '), ' switcher-name ') and normalize-space(.)='${name}']]`,
    );
    await row.waitForExist({ timeout: 10_000 });
    await row.click();
    await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === name, {
      timeout: 10_000, timeoutMsg: `${name} did not open`,
    });
  };
  const shiftOpenCurrentTitle = async (name) => {
    const opened = await browser.execute((expected) => {
      const title = document.querySelector("h1.page-title");
      if (title?.textContent?.trim() !== expected) return false;
      title.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, shiftKey: true }));
      return true;
    }, name);
    if (!opened) throw new Error(`page title missing for ${name}`);
  };
  await navigate("Page A");
  await shiftOpenCurrentTitle("Page A");
  await navigate("Page B");
  await shiftOpenCurrentTitle("Page B");
  await waitItemCount(browser, 2);
}

async function expectBodies(browser, count) {
  try {
    await browser.waitUntil(() => browser.execute((expected) =>
      document.querySelectorAll(".right-sidebar-body > .rs-item > .rs-item-body").length === expected, count), {
      timeout: 5000, timeoutMsg: `right sidebar did not reach ${count} mounted bodies`,
    });
  } catch (error) {
    const state = await browser.execute(() => ({
      items: [...document.querySelectorAll(".right-sidebar-body > .rs-item")].map((item) => ({
        surface: item.getAttribute("data-sidebar-surface"),
        collapsed: item.classList.contains("collapsed"),
        bodies: item.querySelectorAll(":scope > .rs-item-body").length,
        text: item.textContent,
      })),
      bodyCount: document.querySelectorAll(".right-sidebar .rs-item-body").length,
    }));
    throw new Error(`${String(error)}; sidebar state ${JSON.stringify(state)}`);
  }
}

async function openActionsWithKeyboard(browser) {
  await browser.execute(() => document.querySelector("[data-right-sidebar-actions]")?.focus());
  await browser.keys("Enter");
  await browser.$(".rs-actions-menu").waitForExist({ timeout: 5000 });
}

await withApp(0, async (browser) => {
  await openPagesInSidebar(browser);
  await browser.saveScreenshot(`${ARTIFACT}/expanded-items.png`);
  const pageA = await itemSelector("Page A");
  const parentBlock = `${pageA} [data-block-id="sidebar-fold-parent-159"]`;
  const childBlock = `${pageA} [data-block-id="sidebar-fold-child-159"]`;
  const parentToggle = `${parentBlock} > .block-main .collapse-toggle.has-children`;
  await browser.$(parentToggle).waitForExist({ timeout: 10_000 });
  await browser.$(parentToggle).click();
  await browser.waitUntil(() => browser.execute(({ pageSelector, childSelector }) => {
    const item = document.querySelector(pageSelector);
    return !document.querySelector(childSelector)
      && !!item?.querySelector(":scope > .rs-item-body")
      && item?.querySelector("[data-right-sidebar-item-toggle]")?.getAttribute("aria-expanded") === "true";
  }, { pageSelector: pageA, childSelector: childBlock }), {
    timeout: 10_000, timeoutMsg: "sidebar Block disclosure did not hide only its child",
  });
  await browser.$(parentToggle).click();
  await browser.$(childBlock).waitForExist({ timeout: 10_000 });

  await browser.execute((selector) => document.querySelector(`${selector} [data-right-sidebar-item-toggle]`)?.focus(), pageA);
  await browser.keys("Enter");
  await expectBodies(browser, 1);
  const collapsed = await browser.$(`${pageA} [data-right-sidebar-item-toggle]`).getAttribute("aria-expanded");
  if (collapsed !== "false") throw new Error(`keyboard disclosure aria-expanded=${collapsed}`);
  await browser.keys("Enter");
  await expectBodies(browser, 2);

  await browser.$(`${pageA} .block-content`).click();
  const editor = await browser.$(`${pageA} textarea.block-editor`);
  await editor.waitForExist({ timeout: 5000 });
  await browser.execute((selector) => {
    const input = document.querySelector(selector);
    input.focus();
    input.value = "Committed before native collapse";
    input.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
  }, `${pageA} textarea.block-editor`);
  await browser.$(`${pageA} [data-right-sidebar-item-toggle]`).click();
  await expectBodies(browser, 1);
  await browser.waitUntil(() => fs.readFileSync(PAGE_A, "utf8").includes("Committed before native collapse"), {
    timeout: 10_000, timeoutMsg: "sidebar edit was not persisted before body unmount",
  });

  await openActionsWithKeyboard(browser);
  await browser.keys("Enter"); // first menu item: Collapse all
  await expectBodies(browser, 0);
});

// A fresh native process restores both items collapsed from the graph-local
// session, then keyboard-operates Expand all and Close all.
await withApp(1, async (browser) => {
  await waitItemCount(browser, 2);
  await expectBodies(browser, 0);
  await openActionsWithKeyboard(browser);
  await browser.keys(["ArrowDown"]); // Expand all
  await browser.keys("Enter");
  await expectBodies(browser, 2);

  await openActionsWithKeyboard(browser);
  await browser.keys("End"); // Close all
  await browser.keys("Enter");
  await waitItemCount(browser, 0);
  const empty = await browser.$(".rs-empty").getText();
  if (!empty.includes("Nothing open")) throw new Error(`missing empty collection state: ${empty}`);
});

console.log("PASS: sidebar Block disclosure, item disclosures, safe edit unmount, bulk controls, and restart restore work in WebKit");
