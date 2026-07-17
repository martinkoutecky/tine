// Linux real-app proof for GH #84. A fake xdg-open records the direct argv so
// the test verifies exact nested source identity without launching a host app.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4472);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4473);
const TMP = "/tmp/tine-page-file-e2e";
const GRAPH = `${TMP}/graph`;
const SOURCE = `${GRAPH}/pages/nested/Exact.org`;
const CALLS = `${TMP}/opener-calls.txt`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages/nested", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(`${TMP}/bin`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{:preferred-format \"Org\"}\n");
fs.writeFileSync(SOURCE, "* exact nested page\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.org`, "* open [[Exact]]\n");
fs.writeFileSync(`${TMP}/bin/dbus-send`, "#!/bin/sh\nexit 1\n", { mode: 0o755 });
fs.writeFileSync(`${TMP}/bin/xdg-open`, `#!/bin/sh\nprintf '%s\\n' "$@" >> ${JSON.stringify(CALLS)}\n`, { mode: 0o755 });

const env = {
  ...process.env,
  PATH: `${TMP}/bin:${process.env.PATH}`,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
async function activePageAction() {
  return browser.execute(() => document.activeElement?.getAttribute("data-page-action-id") ?? null);
}

async function openTitleActions() {
  const trigger = await browser.$("[data-page-actions-trigger]");
  await trigger.click();
  await browser.$('.ctx-menu[role="menu"]').waitForExist({ timeout: 5_000 });
  await browser.waitUntil(async () => (await activePageAction()) === "open", {
    timeout: 5_000,
    timeoutMsg: "page actions did not focus the first enabled item",
  });
  return trigger;
}

async function focusFavoriteAction() {
  await browser.keys("Home");
  await browser.keys("ArrowDown");
  await browser.keys("ArrowDown");
  await browser.keys("ArrowDown");
  if (await activePageAction() !== "favorite-toggle") {
    throw new Error(`keyboard navigation missed favorite-toggle; active=${await activePageAction()}`);
  }
}

async function runMenu(label) {
  await browser.execute(() => {
    const title = document.querySelector("h1.page-title");
    title.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 100, clientY: 100 }));
  });
  await browser.waitUntil(() => browser.execute((wanted) =>
    [...document.querySelectorAll(".ctx-item")].some((item) => item.textContent?.trim() === wanted), label),
  { timeout: 5000, timeoutMsg: `${label} menu item did not appear` });
  await browser.execute((wanted) => {
    const item = [...document.querySelectorAll(".ctx-item")].find((node) => node.textContent?.trim() === wanted);
    item.click();
  }, label);
}

try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  for (const selector of ["a.page-ref=Exact", "span.page-ref=Exact", "*=Exact"]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  await browser.$("h1.page-title").waitForExist({ timeout: 10_000 });

  // GH #184: a duplicated page surface must expose expanded state only on the
  // exact trigger that owns the global page menu. A title right-click has no
  // ellipsis owner, so neither duplicate trigger may claim it.
  await browser.execute(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", {
      key: "\\", code: "Backslash", ctrlKey: true, altKey: true,
      bubbles: true, cancelable: true,
    }));
  });
  await browser.waitUntil(() => browser.execute(() => {
    const panes = [...document.querySelectorAll(".pane-leaf[data-pane-id]")];
    return panes.length === 2 && panes.every((pane) =>
      pane.querySelector("[data-page-actions-trigger]")
        && pane.querySelector("h1.page-title"));
  }), { timeout: 10_000, timeoutMsg: "duplicate page split was not ready" });
  const [paneA, paneB] = await browser.execute(() =>
    [...document.querySelectorAll(".pane-leaf[data-pane-id]")]
      .map((pane) => pane.getAttribute("data-pane-id")));
  if (!paneA || !paneB || paneA === paneB) {
    throw new Error("duplicate page pane ids were not found");
  }
  const clickPageActions = async (paneId) => {
    const clicked = await browser.execute((id) => {
      const pane = [...document.querySelectorAll(".pane-leaf[data-pane-id]")]
        .find((node) => node.getAttribute("data-pane-id") === id);
      const trigger = pane?.querySelector("[data-page-actions-trigger]");
      if (!trigger) return false;
      trigger.click();
      return true;
    }, paneId);
    if (!clicked) throw new Error(`page-actions trigger for pane ${paneId} was not found`);
  };
  const expandedByPane = () => browser.execute((paneIds) => paneIds.map((id) => {
    const pane = [...document.querySelectorAll(".pane-leaf[data-pane-id]")]
      .find((node) => node.getAttribute("data-pane-id") === id);
    return pane?.querySelector("[data-page-actions-trigger]")?.getAttribute("aria-expanded") ?? null;
  }), [paneA, paneB]);
  const assertExpanded = async (expected, label) => {
    const actual = await expandedByPane();
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(`${label}: ${JSON.stringify(actual)} expected ${JSON.stringify(expected)}`);
    }
  };

  for (const [owner, paneId] of [paneA, paneB].entries()) {
    await clickPageActions(paneId);
    await browser.$('.ctx-menu[role="menu"]').waitForExist({ timeout: 5_000 });
    await assertExpanded([paneA, paneB].map((_, index) => index === owner ? "true" : "false"),
      `page-actions owner mismatch on split pane ${owner + 1}`);
    await browser.keys("Escape");
    await browser.$('.ctx-menu[role="menu"]').waitForExist({ reverse: true, timeout: 5_000 });
    await assertExpanded(["false", "false"], "closed duplicate page-actions state mismatch");
  }

  const titleContextMenuOpened = await browser.execute((paneId) => {
    const pane = [...document.querySelectorAll(".pane-leaf[data-pane-id]")]
      .find((node) => node.getAttribute("data-pane-id") === paneId);
    const title = pane?.querySelector("h1.page-title");
    if (!title) return false;
    title.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true, cancelable: true, clientX: 100, clientY: 100,
    }));
    return true;
  }, paneA);
  if (!titleContextMenuOpened) throw new Error("split page title was not found");
  await browser.$('.ctx-menu[role="menu"]').waitForExist({ timeout: 5_000 });
  await assertExpanded(["false", "false"], "title right-click claimed an ellipsis owner");
  await browser.keys("Escape");
  await browser.$('.ctx-menu[role="menu"]').waitForExist({ reverse: true, timeout: 5_000 });

  await openTitleActions();
  const ids = await browser.execute(() => [...document.querySelectorAll("[data-page-action-id]")]
    .map((item) => item.getAttribute("data-page-action-id")));
  const expectedIds = [
    "open", "open-sidebar", "open-new-tab", "favorite-toggle",
    "copy-page-ref", "copy-page-markdown", "export-pdf",
    "show-in-folder", "open-default-app", "page-properties",
    "rename-page", "delete-page",
  ];
  if (JSON.stringify(ids) !== JSON.stringify(expectedIds)) {
    throw new Error(`page action identity/order mismatch: ${JSON.stringify(ids)}`);
  }
  await browser.keys("ArrowUp");
  if (await activePageAction() !== "delete-page") throw new Error("ArrowUp did not wrap to the last page action");
  await browser.keys("Home");
  if (await activePageAction() !== "open") throw new Error("Home did not focus the first page action");
  await browser.keys("End");
  if (await activePageAction() !== "delete-page") throw new Error("End did not focus the last page action");

  await browser.$('[data-page-action-id="rename-page"]').click();
  await browser.$(".ctx-rename-name").waitForExist({ timeout: 5_000 });
  await browser.keys("Escape");
  await browser.waitUntil(async () => (await activePageAction()) === "rename-page", {
    timeout: 5_000,
    timeoutMsg: "first rename Escape did not keep the menu open and restore the rename item",
  });
  await browser.keys("Escape");
  await browser.$('.ctx-menu[role="menu"]').waitForExist({ reverse: true, timeout: 5_000 });
  const triggerRestored = await browser.execute(() => document.activeElement?.hasAttribute("data-page-actions-trigger") ?? false);
  if (!triggerRestored) throw new Error("second Escape did not restore the page actions trigger");

  await openTitleActions();
  await focusFavoriteAction();
  await browser.keys("Enter");
  await browser.$('.ctx-menu[role="menu"]').waitForExist({ reverse: true, timeout: 5_000 });
  if (!(await browser.$(".fav-star").getAttribute("class")).includes("active")) {
    throw new Error("Enter did not activate favorite-toggle exactly once");
  }
  await openTitleActions();
  await focusFavoriteAction();
  await browser.keys(" ");
  await browser.$('.ctx-menu[role="menu"]').waitForExist({ reverse: true, timeout: 5_000 });
  if ((await browser.$(".fav-star").getAttribute("class")).includes("active")) {
    throw new Error("Space did not activate favorite-toggle exactly once");
  }

  await browser.setWindowSize(360, 640);
  await openTitleActions();
  const narrowGeometry = await browser.execute(() => {
    const menu = document.querySelector('.ctx-menu[role="menu"]')?.getBoundingClientRect();
    const trigger = document.querySelector("[data-page-actions-trigger]")?.getBoundingClientRect();
    return menu && trigger ? {
      innerWidth,
      innerHeight,
      menu: { left: menu.left, top: menu.top, right: menu.right, bottom: menu.bottom },
      trigger: { width: trigger.width, height: trigger.height },
    } : null;
  });
  if (!narrowGeometry
      || narrowGeometry.menu.left < 0
      || narrowGeometry.menu.top < 0
      || narrowGeometry.menu.right > narrowGeometry.innerWidth
      || narrowGeometry.menu.bottom > narrowGeometry.innerHeight) {
    throw new Error(`360px page actions escaped the viewport: ${JSON.stringify(narrowGeometry)}`);
  }
  await browser.keys("Escape");
  await browser.setWindowSize(1000, 720);

  await runMenu("Show in folder");
  await browser.waitUntil(() => fs.existsSync(CALLS), { timeout: 5000 });
  await runMenu("Open with default app");
  await browser.waitUntil(() => fs.readFileSync(CALLS, "utf8").trim().split("\n").length >= 2, { timeout: 5000 });
  const calls = fs.readFileSync(CALLS, "utf8").trim().split("\n");
  if (calls[0] !== path.dirname(SOURCE) || calls[1] !== SOURCE) throw new Error(`wrong opener argv: ${JSON.stringify(calls)}`);
  console.log(`PASS: accessible ellipsis keyboard/focus flow and title right-click used exact source actions ${calls[0]} / ${calls[1]}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
