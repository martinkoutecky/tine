// Literal Linux/Tauri/WebKit acceptance for GH #161. This drives a release
// binary twice: the Rust-only 390px main-window fixture, then an ordinary 960px
// desktop/tablet neighbor. Assertions observe the production shell and editor;
// no frontend signal is mutated by the harness.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync(path.join(process.env.TMPDIR || "/tmp", "tine-mobile-drawers-e2e-"));
const GRAPH = path.join(TMP, "graph");
const PAGE = path.join(GRAPH, "pages", "Drawer target.md");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts"));
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4582);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4583);
const EDIT_ID = "drawer-edit-161";
const SAVED_TEXT = "Native saved [[Completion Target]] 中文";
const PDF = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA1IDAgUiA+PiA+PiAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAyMDUgPj4Kc3RyZWFtCkJUIC9GMSAyMCBUZiA3MiA3MjAgVGQgKFRpbmUgUERGIHZpZXdlcikgVGogRVQKQlQgL0YxIDEzIFRmIDcyIDY5MCBUZCAoU2VsZWN0IHRoaXMgdGV4dCB0byBjcmVhdGUgYSBoaWdobGlnaHQuKSBUaiBFVApCVCAvRjEgMTMgVGYgNzIgNjY4IFRkIChIaWdobGlnaHRzIHBlcnNpc3QgdG8gYXNzZXRzLzxrZXk+LmVkbiArIGFuIGhsc19fIHBhZ2UuKSBUaiBFVAoKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKeHJlZgowIDYKMDAwMDAwMDAwMCA2NTUzNSBmIAowMDAwMDAwMDA5IDAwMDAwIG4gCjAwMDAwMDAwNTggMDAwMDAgbiAKMDAwMDAwMDExNSAwMDAwMCBuIAowMDAwMDAwMjQxIDAwMDAwIG4gCjAwMDAwMDA0OTcgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA2IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgo1NjcKJSVFT0Y=";

for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(ARTIFACT, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), '{:favorites ["Drawer target"]}\n');
fs.writeFileSync(PAGE, [
  "- Sidebar original",
  `  id:: ${EDIT_ID}`,
  "- [[Underlying route]]",
  "- [Fixture PDF](../assets/drawer-fixture.pdf)",
  "",
].join("\n"));
fs.writeFileSync(path.join(GRAPH, "pages", "Completion Target.md"), "- completion fixture\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Underlying route.md"), "- this route must not open through the scrim\n");
fs.writeFileSync(path.join(GRAPH, "assets", "drawer-fixture.pdf"), Buffer.from(PDF, "base64"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), "- Drawer fixture home\n");

const proof = {
  scenario: "gh-161-native-mobile-drawers",
  app: APP,
  graph: GRAPH,
  startedAt: new Date().toISOString(),
  forced: {},
  restart: {},
  regularWidth: {},
  artifacts: {},
};

function assert(condition, message, detail) {
  if (!condition) throw new Error(`${message}${detail === undefined ? "" : `: ${JSON.stringify(detail)}`}`);
}

function baseEnv(forced) {
  const env = {
    ...process.env,
    TINE_GRAPH: GRAPH,
    XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
    XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
    XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    LIBGL_ALWAYS_SOFTWARE: "1",
    GDK_BACKEND: "x11",
  };
  if (forced) env.TINE_E2E_FORCE_MOBILE_DRAWERS = "1";
  else delete env.TINE_E2E_FORCE_MOBILE_DRAWERS;
  return env;
}

async function withApp(index, forced, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const logPath = path.join(ARTIFACT, `driver-${index}-${forced ? "forced" : "regular"}.log`);
  const log = fs.openSync(logPath, "w");
  proof.artifacts[`driver${index}`] = logPath;
  const driver = spawn(DRIVER, [
    "--port", String(driverPort),
    "--native-port", String(nativePort),
    "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
  ], { env: baseEnv(forced), detached: true, stdio: ["ignore", log, log] });
  let browser;
  try {
    await sleep(2500);
    browser = await remote({
      hostname: "127.0.0.1",
      port: driverPort,
      path: "/",
      logLevel: "error",
      connectionRetryCount: 1,
      connectionRetryTimeout: 60_000,
      capabilities: {
        browserName: "wry",
        "wdio:enforceWebDriverClassic": true,
        "tauri:options": { application: APP },
      },
    });
    await browser.$(".app-container").waitForExist({ timeout: 20_000 });
    await browser.$(".main-content").waitForExist({ timeout: 20_000 });
    await fn(browser);
    await sleep(500);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    try { fs.closeSync(log); } catch {}
  }
}

async function snapshot(browser) {
  return browser.execute(() => {
    const rect = (selector) => {
      const node = document.querySelector(selector);
      if (!(node instanceof HTMLElement)) return null;
      const r = node.getBoundingClientRect();
      return { x: r.x, y: r.y, width: r.width, height: r.height, right: r.right, bottom: r.bottom };
    };
    const style = (selector) => {
      const node = document.querySelector(selector);
      return node instanceof HTMLElement ? getComputedStyle(node).display : "missing";
    };
    const active = document.activeElement;
    return {
      viewport: { width: innerWidth, height: innerHeight },
      mode: document.querySelector(".app-container")?.getAttribute("data-mobile-drawer-mode"),
      activeDrawer: document.querySelector(".app-container")?.getAttribute("data-active-drawer"),
      title: document.querySelector("h1.page-title")?.textContent?.trim() ?? null,
      main: rect(".main-content"),
      mainContainer: rect(".main-container"),
      workspace: rect(".drawer-workspace"),
      left: rect(".left-sidebar"),
      right: rect(".right-sidebar"),
      pdf: rect(".pdf-pane"),
      panes: [...document.querySelectorAll("[data-pane-id]")].map((node) => {
        const r = node.getBoundingClientRect();
        return { id: node.getAttribute("data-pane-id"), x: r.x, width: r.width, right: r.right };
      }),
      scrims: document.querySelectorAll(".mobile-drawer-scrim").length,
      panels: document.querySelectorAll("[data-mobile-drawer-panel]").length,
      leftRole: document.querySelector(".left-sidebar")?.getAttribute("role") ?? null,
      leftModal: document.querySelector(".left-sidebar")?.getAttribute("aria-modal") ?? null,
      rightRole: document.querySelector(".right-sidebar")?.getAttribute("role") ?? null,
      rightModal: document.querySelector(".right-sidebar")?.getAttribute("aria-modal") ?? null,
      inert: [...document.querySelectorAll("[data-drawer-background][inert]")].map((node) => node.getAttribute("class")),
      active: active instanceof HTMLElement ? {
        tag: active.tagName,
        className: active.className,
        inLeft: Boolean(active.closest(".left-sidebar")),
        inRight: Boolean(active.closest(".right-sidebar")),
        inMain: Boolean(active.closest(".main-content")),
        title: active.getAttribute("title"),
      } : null,
      resizers: { left: style(".sidebar-resizer"), right: style(".rs-resizer") },
      selectedBlocks: document.querySelectorAll(".block-selected, .selected-block").length,
    };
  });
}

async function pointerAt(browser, x, y) {
  await browser.performActions([{
    type: "pointer",
    id: "native-mouse",
    parameters: { pointerType: "mouse" },
    actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", x: Math.round(x), y: Math.round(y) },
      { type: "pointerDown", button: 0 },
      { type: "pointerUp", button: 0 },
    ],
  }]);
  await browser.releaseActions();
}

function nativeScreenshot(file) {
  // WebKitGTK's screenshot endpoint can hang after painting a live nested
  // sidebar editor. Capture the actual isolated Xvfb surface instead; this is
  // also the native pixels a user sees, and avoids turning an observation into
  // a WebDriver transport flake.
  const result = spawnSync("import", ["-window", "root", file], { env: process.env, encoding: "utf8" });
  assert(result.status === 0 && fs.existsSync(file) && fs.statSync(file).size > 0,
    "native X11 screenshot failed", { file, status: result.status, stderr: result.stderr });
}

async function waitDrawer(browser, side) {
  await browser.waitUntil(() => browser.execute((wanted) =>
    document.querySelector(".app-container")?.getAttribute("data-active-drawer") === wanted
    && document.querySelectorAll(".mobile-drawer-scrim").length === 1,
  side), { timeout: 8_000, timeoutMsg: `${side} drawer did not become active` });
}

async function waitNoDrawer(browser) {
  await browser.waitUntil(() => browser.execute(() =>
    !document.querySelector(".app-container")?.getAttribute("data-active-drawer")
    && document.querySelectorAll(".mobile-drawer-scrim").length === 0),
  { timeout: 8_000, timeoutMsg: "drawer/scrim did not close" });
}

async function clickToolbar(browser, title) {
  const button = await browser.$(`.icon-btn[title^='${title}']`);
  await button.waitForExist({ timeout: 5_000 });
  await button.click();
}

async function shiftClick(browser, element) {
  await browser.performActions([{
    type: "key",
    id: "native-keyboard",
    actions: [{ type: "keyDown", value: "\uE008" }],
  }]);
  try { await element.click(); } finally { await browser.releaseActions(); }
}

async function navigate(browser, name) {
  const current = await browser.$("h1.page-title");
  if (await current.isExisting() && (await current.getText()).trim() === name) return;
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue(name);
  await browser.waitUntil(() => browser.execute((wanted) =>
    [...document.querySelectorAll(".switcher-row .switcher-name")].some((node) => node.textContent?.trim() === wanted), name),
  { timeout: 10_000, timeoutMsg: `${name} was not offered by Quick switcher` });
  const clicked = await browser.execute((wanted) => {
    const label = [...document.querySelectorAll(".switcher-row .switcher-name")]
      .find((node) => node.textContent?.trim() === wanted);
    const row = label?.closest(".switcher-row");
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  }, name);
  assert(clicked, `could not activate ${name} Quick switcher row`);
  await browser.waitUntil(async () => {
    const heading = await browser.$("h1.page-title");
    return await heading.isExisting() && (await heading.getText()).trim() === name;
  }, { timeout: 10_000, timeoutMsg: `${name} did not open` });
}

// Forced phone-width native process: literal drawer geometry and behavior.
await withApp(0, true, async (browser) => {
  const initial = await snapshot(browser);
  assert(initial.viewport.width >= 370 && initial.viewport.width <= 410, "forced native width missed bounded 390px target", initial.viewport);
  assert(initial.viewport.width < 640 && initial.mode === "true", "forced process was not classified by its actual sub-640 viewport", initial);

  if (initial.scrims) {
    await browser.$(".mobile-drawer-close, .rs-close").click();
    await waitNoDrawer(browser);
  }
  const closed = await snapshot(browser);

  await clickToolbar(browser, "Toggle sidebar");
  await waitDrawer(browser, "left");
  const left = await snapshot(browser);
  assert(Math.abs(left.main.x - closed.main.x) <= 1 && Math.abs(left.main.width - closed.main.width) <= 1,
    "left drawer squeezed or shifted the main workspace", { closed: closed.main, open: left.main });
  assert(left.left.x <= 1 && left.left.width <= initial.viewport.width - 44 + 1,
    "left drawer was not edge-anchored/capped", left.left);
  assert(left.scrims === 1 && left.panels === 1 && left.leftRole === "dialog" && left.leftModal === "true",
    "left drawer omitted its one modal panel/scrim", left);
  assert(left.resizers.left === "none" && left.active?.inLeft,
    "left drawer exposed a resizer or failed to own focus", left);
  proof.artifacts.left = path.join(ARTIFACT, "forced-left.png");
  nativeScreenshot(proof.artifacts.left);

  // A genuine favorite navigation is successful only after the new page is
  // visible; it must close the left drawer and focus the active main pane.
  const favorite = await browser.$("#sidebar-favorites-list .nav-page");
  await favorite.waitForExist({ timeout: 10_000 });
  await favorite.click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Drawer target", {
    timeout: 10_000, timeoutMsg: "favorite did not navigate to Drawer target",
  });
  await waitNoDrawer(browser);
  const leftNavigation = await snapshot(browser);
  assert(leftNavigation.active?.inMain, "successful left navigation did not focus main content", leftNavigation.active);

  // Open a real page in the right sidebar through the established Shift-click
  // gesture, then verify overlay geometry and inert focus containment.
  await shiftClick(browser, await browser.$("h1.page-title"));
  await waitDrawer(browser, "right");
  await browser.$(`[data-sidebar-surface] [data-block-id='${EDIT_ID}']`).waitForExist({ timeout: 10_000 });
  const right = await snapshot(browser);
  assert(Math.abs(right.main.x - closed.main.x) <= 1 && Math.abs(right.main.width - closed.main.width) <= 1,
    "right drawer squeezed or shifted the main workspace", { closed: closed.main, open: right.main });
  assert(right.right.right >= initial.viewport.width - 1 && right.right.width <= initial.viewport.width - 44 + 1,
    "right drawer was not edge-anchored/capped", right.right);
  assert(right.scrims === 1 && right.panels === 1 && right.rightRole === "dialog" && right.rightModal === "true"
    && right.resizers.right === "none" && right.inert.length > 0 && right.active?.inRight,
  "right drawer omitted modal geometry, inert isolation, or focus ownership", right);

  const rejectedFocus = await browser.execute(() => {
    const target = document.querySelector(".icon-btn[title^='Toggle sidebar']");
    if (!(target instanceof HTMLElement)) return null;
    target.focus();
    const active = document.activeElement;
    return {
      targetOwnsFocus: active === target,
      activeInRight: active instanceof HTMLElement && Boolean(active.closest(".right-sidebar")),
    };
  });
  assert(rejectedFocus && !rejectedFocus.targetOwnsFocus && rejectedFocus.activeInRight,
    "inert background focus attempt escaped the right drawer", rejectedFocus);
  proof.artifacts.right = path.join(ARTIFACT, "forced-right.png");
  nativeScreenshot(proof.artifacts.right);

  // Click the real screen coordinates of the inert left toolbar toggle. The
  // scrim is the hit target; it may dismiss the right drawer, but must not open
  // the left drawer or change the active page underneath.
  const underlay = await browser.execute(() => {
    const target = document.querySelector(".icon-btn[title^='Toggle sidebar']");
    const rect = target?.getBoundingClientRect();
    if (!rect) return null;
    const x = rect.x + rect.width / 2;
    const y = rect.y + rect.height / 2;
    const hit = document.elementFromPoint(x, y);
    return { x, y, hitClass: hit?.className ?? null, hitIsScrim: Boolean(hit?.closest(".mobile-drawer-scrim")) };
  });
  assert(underlay?.hitIsScrim, "underlying activation target was not genuinely covered by the scrim", underlay);
  await pointerAt(browser, underlay.x, underlay.y);
  await waitNoDrawer(browser);
  const consumed = await snapshot(browser);
  assert(consumed.title === "Drawer target" && !consumed.left && !consumed.right && consumed.active?.inMain,
    "scrim pointer activated the covered toolbar target or failed focus restoration", consumed);

  // Escape peels the actions menu first and the drawer second, restoring the
  // actual toolbar opener only on the second key.
  await clickToolbar(browser, "Toggle right sidebar");
  await waitDrawer(browser, "right");
  await browser.$("[data-right-sidebar-actions]").click();
  await browser.$(".rs-actions-menu").waitForExist({ timeout: 5_000 });
  await browser.keys(["Escape"]);
  await browser.$(".rs-actions-menu").waitForExist({ timeout: 5_000, reverse: true });
  const menuEscaped = await snapshot(browser);
  assert(menuEscaped.activeDrawer === "right" && menuEscaped.active?.className.includes("rs-actions-button"),
    "first Escape did not close only the right-sidebar actions menu", menuEscaped);
  await browser.keys(["Escape"]);
  await waitNoDrawer(browser);
  const drawerEscaped = await snapshot(browser);
  assert(drawerEscaped.active?.title?.startsWith("Toggle right sidebar") && drawerEscaped.title === "Drawer target",
    "second Escape did not restore the drawer opener without navigation", drawerEscaped);

  // Literal right-sidebar editor: native completion gets the first Escape,
  // synthetic composition Escape is ignored, then plain Escape closes the
  // whole drawer, commits, and never enters block/pane selection.
  await clickToolbar(browser, "Toggle right sidebar");
  await waitDrawer(browser, "right");
  const contentSelector = `.right-sidebar [data-block-id='${EDIT_ID}'] .block-content`;
  await browser.$(contentSelector).click();
  const editor = await browser.$(`.right-sidebar [data-block-id='${EDIT_ID}'] textarea.block-editor`);
  await editor.waitForExist({ timeout: 5_000 });
  await editor.setValue("Native plain [[Comp");
  await browser.$(".autocomplete .ac-item").waitForExist({ timeout: 10_000 });
  await browser.keys(["Escape"]);
  await browser.$(".autocomplete").waitForExist({ timeout: 5_000, reverse: true });
  const completionEscaped = await snapshot(browser);
  assert(completionEscaped.activeDrawer === "right" && completionEscaped.active?.tag === "TEXTAREA" && completionEscaped.active?.inRight,
    "completion Escape also closed the editor/drawer", completionEscaped);
  await editor.setValue(SAVED_TEXT);
  const composition = await browser.execute(() => {
    const textarea = document.activeElement;
    if (!(textarea instanceof HTMLTextAreaElement)) return { dispatched: false };
    textarea.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
    const event = new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true, composed: true });
    Object.defineProperty(event, "isComposing", { configurable: true, value: true });
    textarea.dispatchEvent(event);
    const result = {
      dispatched: true,
      defaultPrevented: event.defaultPrevented,
      drawer: document.querySelector(".app-container")?.getAttribute("data-active-drawer"),
      editorActive: document.activeElement === textarea,
    };
    textarea.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "文" }));
    return result;
  });
  assert(composition.dispatched && composition.drawer === "right" && composition.editorActive,
    "composition-state Escape closed the editor/drawer", composition);
  await browser.keys(["Escape"]);
  await waitNoDrawer(browser);
  const plainEscaped = await snapshot(browser);
  assert(plainEscaped.selectedBlocks === 0, "plain right-editor Escape entered block/pane selection", plainEscaped);
  await browser.waitUntil(() => fs.readFileSync(PAGE, "utf8").includes(SAVED_TEXT), {
    timeout: 12_000, timeoutMsg: "whole-drawer close did not persist the live right-sidebar edit to disk",
  });

  await clickToolbar(browser, "Toggle right sidebar");
  await waitDrawer(browser, "right");
  const reopenedText = await browser.$(`.right-sidebar [data-block-id='${EDIT_ID}'] .block-content`).getText();
  assert(reopenedText.includes("Native saved") && reopenedText.includes("中文"), "saved edit did not survive drawer reopen", reopenedText);

  // Real shortcuts must preserve one-panel/one-scrim exclusivity in both
  // directions, including a right -> left return.
  await browser.keys(["t", "l"]);
  await waitDrawer(browser, "left");
  const exclusiveLeft = await snapshot(browser);
  await browser.keys(["t", "r"]);
  await waitDrawer(browser, "right");
  const exclusiveRight = await snapshot(browser);
  await browser.keys(["t", "l"]);
  await waitDrawer(browser, "left");
  const exclusiveLeftAgain = await snapshot(browser);
  for (const state of [exclusiveLeft, exclusiveRight, exclusiveLeftAgain]) {
    assert(state.panels === 1 && state.scrims === 1, "drawer switch left stale panels/scrims", state);
  }
  await browser.keys(["Escape"]);
  await waitNoDrawer(browser);

  proof.forced = {
    viewport: initial.viewport,
    classifier: initial.mode,
    closedMain: closed.main,
    left,
    leftNavigation,
    right,
    rejectedFocus,
    underlay,
    consumed,
    menuEscaped,
    drawerEscaped,
    completionEscaped,
    composition,
    plainEscaped,
    diskSaved: fs.readFileSync(PAGE, "utf8").includes(SAVED_TEXT),
    exclusivity: [exclusiveLeft.activeDrawer, exclusiveRight.activeDrawer, exclusiveLeftAgain.activeDrawer],
  };
});

// Same XDG/graph, fresh native process: persistence must not be an immediate
// DOM illusion.
await withApp(1, true, async (browser) => {
  const start = await snapshot(browser);
  assert(start.viewport.width < 640 && start.mode === "true", "restart lost literal phone-width classification", start);
  if (start.activeDrawer) {
    await browser.keys(["Escape"]);
    await waitNoDrawer(browser);
  }
  await navigate(browser, "Drawer target");
  await clickToolbar(browser, "Toggle right sidebar");
  await waitDrawer(browser, "right");
  const item = await browser.$(`.right-sidebar [data-block-id='${EDIT_ID}'] .block-content`);
  await item.waitForExist({ timeout: 10_000 });
  const text = await item.getText();
  assert(text.includes("Native saved") && text.includes("中文"), "disk edit/right-sidebar item did not survive native restart", text);
  proof.artifacts.restart = path.join(ARTIFACT, "forced-restart.png");
  nativeScreenshot(proof.artifacts.restart);
  proof.restart = { viewport: start.viewport, classifier: start.mode, restoredText: text, disk: fs.readFileSync(PAGE, "utf8") };
});

// Unforced >=640 neighbor: the exact same build must use persistent desktop/
// tablet sidebars, simultaneous layout widths, and retain split/PDF siblings.
await withApp(2, false, async (browser) => {
  await browser.setWindowSize(960, 760);
  await browser.waitUntil(() => browser.execute(() => innerWidth >= 900), {
    timeout: 8_000, timeoutMsg: "ordinary native window did not reach the requested regular width",
  });
  let state = await snapshot(browser);
  assert(state.viewport.width >= 640 && state.mode === "false", "unforced regular viewport entered drawer mode", state);

  if (state.left) await clickToolbar(browser, "Toggle sidebar");
  state = await snapshot(browser);
  if (state.right) await clickToolbar(browser, "Toggle right sidebar");
  const regularClosed = await snapshot(browser);
  await clickToolbar(browser, "Toggle sidebar");
  await browser.$(".left-sidebar").waitForExist({ timeout: 5_000 });
  await clickToolbar(browser, "Toggle right sidebar");
  await browser.$(".right-sidebar").waitForExist({ timeout: 5_000 });
  const regularBoth = await snapshot(browser);
  assert(regularBoth.left && regularBoth.right && regularBoth.scrims === 0 && regularBoth.inert.length === 0,
    "regular width did not permit simultaneous persistent sidebars", regularBoth);
  assert(regularBoth.leftRole === null && regularBoth.rightRole === null && regularBoth.leftModal === null && regularBoth.rightModal === null,
    "regular sidebars retained mobile dialog semantics", regularBoth);
  assert(regularBoth.resizers.left !== "none" && regularBoth.resizers.right !== "none",
    "regular sidebars hid their resize seams", regularBoth.resizers);
  assert(regularBoth.mainContainer.x >= regularBoth.left.width - 1
    && regularBoth.mainContainer.width < regularClosed.mainContainer.width
    && regularBoth.workspace.width < regularClosed.workspace.width,
  "regular sidebars did not consume their persisted flex widths", { closed: regularClosed, both: regularBoth });

  // The 960px state above proves both persisted sidebars simultaneously. Give
  // the split+560px PDF neighbor physically meaningful room for its separate
  // structural assertion; keeping every optional surface open inside 960px
  // would test an impossible sum of fixed minimum widths, not drawer parity.
  await clickToolbar(browser, "Toggle sidebar");
  await browser.setWindowSize(1600, 900);
  await browser.waitUntil(() => browser.execute(() => innerWidth >= 1550), {
    timeout: 8_000, timeoutMsg: "regular neighbor did not widen for split/PDF proof",
  });
  await navigate(browser, "Drawer target");
  const rightBeforeNeighbors = (await snapshot(browser)).right;
  await browser.keys(["Control", "Alt", "\\"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll("[data-pane-id]").length === 2), {
    timeout: 8_000, timeoutMsg: "regular-width split neighbor did not mount",
  });
  const split = await snapshot(browser);
  assert(split.panes.length === 2 && split.panes.every((pane) => pane.x >= split.workspace.x - 1 && pane.right <= split.workspace.right + 1)
    && Math.abs(split.right.x - rightBeforeNeighbors.x) <= 1,
  "persistent sidebar restructuring displaced the split neighbor", split);

  const pdfLink = await browser.$(".pdf-link");
  await pdfLink.waitForExist({ timeout: 10_000 });
  await pdfLink.click();
  await browser.$(".pdf-pane").waitForExist({ timeout: 10_000 });
  const pdf = await snapshot(browser);
  const pdfParent = await browser.execute(() => document.querySelector(".pdf-pane")?.parentElement?.classList.contains("drawer-workspace") ?? false);
  assert(pdfParent && pdf.pdf.x >= pdf.workspace.x - 1 && pdf.pdf.right <= pdf.workspace.right + 1
    && Math.abs(pdf.right.x - rightBeforeNeighbors.x) <= 1,
  "persistent sidebar restructuring displaced the PDF neighbor", pdf);
  proof.artifacts.regular = path.join(ARTIFACT, "regular-wide-split-pdf.png");
  // Xvfb's root surface is only 1280px wide even though WebKit can own a wider
  // window. Use the WebDriver window capture here so the 1600px visual receipt
  // includes both 340px split panes, the PDF, and the persistent sidebar.
  await browser.saveScreenshot(proof.artifacts.regular);
  proof.regularWidth = { closed: regularClosed, simultaneous: regularBoth, split, pdf, pdfParent };
});

proof.finishedAt = new Date().toISOString();
proof.result = "pass";
const proofPath = path.join(ARTIFACT, "proof.json");
fs.writeFileSync(proofPath, `${JSON.stringify(proof, null, 2)}\n`);
console.log(`PASS: native <640 drawer flow, restart persistence, and unforced >=640 persistent split/PDF neighbor; proof: ${proofPath}`);
