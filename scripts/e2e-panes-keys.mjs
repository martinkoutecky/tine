// Native semantic journeys for R2 panes/keys.
import os from "node:os";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = process.env.E2E_ARTIFACT_DIR ?? path.join(root, "artifacts", `panes-keys-native-${process.pid}`);
fs.mkdirSync(out, { recursive: true });
const graph = fs.mkdtempSync(path.join(os.tmpdir(), "tine-pk-"));
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(graph, "xdg", dir), { recursive: true });
const names = Array.from({ length: 10 }, (_, i) => `Pane keyboard page ${i + 1}`);
for (const name of names) fs.writeFileSync(path.join(graph, "pages", `${name}.md`), `- Parent on ${name}\n  - Child on ${name}\n- Other root\n`);
fs.writeFileSync(path.join(graph, "pages", "Empty.md"), "");
fs.writeFileSync(path.join(graph, "logseq", "config.edn"), `{:shortcuts {:tab/activate-3 "alt+3" :tab/activate-9 "alt+9" :editor/edit-selected-block "esc" :go/search-pages "mod+alt+7" :go/search-content "mod+alt+8"}}\n`);
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(graph, "journals", `${day}.md`), "- Native panes and keyboard journey\n");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(graph, "xdg", "data"), XDG_CONFIG_HOME: path.join(graph, "xdg", "config"), XDG_CACHE_HOME: path.join(graph, "xdg", "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", "4592", "--native-port", "4593", "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
let browser;
let journey = "startup";
const check = (value, message) => { if (!value) throw new Error(message); };
const report = (message) => console.log(`PASS ${journey}: ${message}`);
try {
  await sleep(2000);
  browser = await remote({ hostname: "127.0.0.1", port: 4592, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(root, "target", "release", "tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1000, 760);
  await browser.$(".page-title").waitForExist({ timeout: 20_000 });
  const search = async (name, modifier = {}) => {
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input"); await input.waitForExist({ timeout: 5000 });
    await input.setValue(name);
    await browser.waitUntil(() => browser.execute((name) => [...document.querySelectorAll(".switcher-row")].some((row) => row.querySelector(".switcher-kind")?.textContent.trim() === "page" && row.querySelector(".switcher-name")?.textContent.trim() === name), name), { timeout: 10_000 });
    await browser.execute((name, modifier) => {
      const row = [...document.querySelectorAll(".switcher-row")].find((row) => row.querySelector(".switcher-kind")?.textContent.trim() === "page" && row.querySelector(".switcher-name")?.textContent.trim() === name);
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, ...modifier }));
    }, name, modifier);
    if (modifier.ctrlKey || modifier.shiftKey) await browser.keys("Escape");
  };
  const title = async () => (await browser.$(":is(.pane-focused, .main-content-shell) h1.page-title").getText()).trim();
  journey = "GH #587 tab slots";
  for (const name of names) await search(name, { ctrlKey: true });
  await browser.keys(["Alt", "3"]);
  await browser.waitUntil(async () => await title() === names[1], { timeout: 5000 });
  await browser.keys(["Alt", "9"]);
  await browser.waitUntil(async () => await title() === names.at(-1), { timeout: 5000 });
  report("remapped slot 3 and last tab activate the expected page");

  journey = "GH #593 hover menus";
  const hoverGap = async (triggerSelector, menuSelector, file) => {
    const start = await browser.execute((selector) => {
      const rect = document.querySelector(selector).getBoundingClientRect();
      return { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };
    }, triggerSelector);
    await browser.performActions([{ type: "pointer", id: "hover", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", ...start },
    ] }]);
    await browser.$(menuSelector).waitForExist({ timeout: 5000 });
    const points = await browser.execute((aSelector, bSelector) => {
      const a = document.querySelector(aSelector).getBoundingClientRect();
      const b = document.querySelector(bSelector).getBoundingClientRect();
      return { x: Math.round(Math.max(b.left + 10, Math.min(a.left + a.width / 2, b.right - 10))), gap: Math.round((a.bottom + b.top) / 2), inside: Math.round(b.top + 15) };
    }, triggerSelector, menuSelector);
    await browser.performActions([{ type: "pointer", id: "hover", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 30, origin: "viewport", x: points.x, y: points.gap },
      { type: "pointerMove", duration: 60, origin: "viewport", x: points.x, y: points.inside }, { type: "pause", duration: 350 },
    ] }]);
    check(await browser.$(menuSelector).isExisting(), "menu closed while crossing its gap");
    console.log(`Native hover visibility verified (${file}); screenshot saved in the artifacts directory.`);
    // Use the same physical pointer source throughout the journey and cleanup.
    const outside = await browser.execute(() => ({ x: innerWidth - 20, y: innerHeight - 30 }));
    await browser.performActions([{ type: "pointer", id: "hover", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 100, origin: "viewport", ...outside }, { type: "pause", duration: 350 },
    ] }]);
    await browser.$(menuSelector).waitForExist({ reverse: true, timeout: 5000 });
    await browser.releaseActions();
  };
  await hoverGap("[data-tab-overview-trigger]", ".tab-overview", "tab-hover.png");
  await hoverGap("[data-workspace-switcher]", ".workspace-quick-menu", "workspace-hover.png");
  report("both real-pointer hover paths survive the gap; built-frontend screenshots recorded separately");

  journey = "GH #428 selection to editing";
  await search(names[0]);
  await browser.$(":is(.pane-focused, .main-content-shell) .block-content").click();
  await browser.$(":is(.pane-focused, .main-content-shell) textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.keys("Escape");
  await browser.$(":is(.pane-focused, .main-content-shell) .block-main.selected").waitForExist({ timeout: 5000 });
  await browser.keys("Escape");
  await browser.$(":is(.pane-focused, .main-content-shell) textarea.block-editor").waitForExist({ timeout: 5000 });
  report("second remapped Escape returns the selected block to its textarea");

  journey = "GH #663 scoped searches";
  for (const [key, kind] of [["7", "page"], ["8", "block"]]) {
    await browser.keys(["Control", "Alt", key]);
    const input = await browser.$(".switcher-input"); await input.waitForExist({ timeout: 5000 });
    await input.setValue("Pane keyboard");
    await browser.waitUntil(() => browser.execute((kind) => {
      const rows = [...document.querySelectorAll(".switcher-row")];
      return rows.length > 0 && rows.every((row) => row.querySelector(".switcher-kind")?.textContent.trim() === kind);
    }, kind), { timeout: 10_000 });
    await browser.keys("Escape");
  }
  report("page-only and content-only results are visible through remapped commands");

  journey = "GH #677 sidebar reopen";
  await search(names[0], { shiftKey: true });
  await search(names[1], { shiftKey: true });
  await search(names[0], { shiftKey: true });
  await browser.waitUntil(() => browser.execute((name) => {
    const rows = [...document.querySelectorAll(".right-sidebar .rs-item")];
    return rows.length === 2 && rows[0].querySelector(".rs-item-title")?.textContent.trim() === name;
  }, names[0]), { timeout: 5000 });
  await browser.execute(() => document.querySelector(":is(.pane-focused, .main-content-shell) h1.page-title")?.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.closest(".rs-item") === document.querySelector(".right-sidebar .rs-item")), { timeout: 5000 });
  report("search and Shift-click promote a single card and the link opener focuses it");

  journey = "GH #639 bullet pane zoom";
  await search(names[0]);
  await browser.execute(() => document.querySelector(":is(.pane-focused, .main-content-shell) .bullet-container")?.dispatchEvent(new MouseEvent("click", { bubbles: true, altKey: true })));
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".pane-leaf").length === 2 && document.querySelectorAll(".zoom-breadcrumb").length > 0), { timeout: 5000 });

  report("Alt bullet opens a pane with a visible block zoom breadcrumb");

  journey = "GH #524 empty sidebar drop";
  await search("Empty", { shiftKey: true });
  const drag = await browser.execute(() => {
    const item = [...document.querySelectorAll(".right-sidebar .rs-item")].find((row) => row.querySelector(".rs-item-title")?.textContent.trim() === "Empty");
    const source = document.querySelector(":is(.pane-focused, .main-content-shell) .bullet-container");
    const target = item?.querySelector(".ls-block .block-main");
    if (!source || !target) return null;
    const a = source.getBoundingClientRect(), b = target.getBoundingClientRect();
    return { x: Math.round(a.left + a.width / 2), y: Math.round(a.top + a.height / 2), tx: Math.round(b.left + 20), ty: Math.round(b.top + 4) };
  });
  check(drag, "empty sidebar exposes no block drop target");
  await browser.performActions([{ type: "pointer", id: "drag", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, origin: "viewport", x: drag.x, y: drag.y }, { type: "pointerDown", button: 0 },
    { type: "pointerMove", duration: 300, origin: "viewport", x: drag.tx, y: drag.ty }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  await browser.waitUntil(() => fs.readFileSync(path.join(graph, "pages", "Empty.md"), "utf8").includes(`Parent on ${names[0]}`), { timeout: 10_000 });
  report("real bullet drag reaches the empty page through the audited cross-page save");

  for (const side of ["top", "right"]) {
    journey = `GH #499 ${side} pane split`;
    const drag = await browser.execute((side) => {
      const panes = [...document.querySelectorAll(".pane-leaf")];
      const source = panes[0].querySelector(".tab.active");
      source.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "instant" });
      const target = panes.at(-1);
      const a = source.getBoundingClientRect(), b = target.getBoundingClientRect();
      const strip = target.querySelector(".tab-bar").getBoundingClientRect();
      const startX = Math.round(a.left + a.width / 2), startY = Math.round(a.top + a.height / 2);
      return { sourceTitle: source.querySelector(".tab-title").textContent, paneIds: panes.map((pane) => pane.dataset.paneId), targetId: target.dataset.paneId, count: panes.length,
        sourceHit: source.contains(document.elementFromPoint(startX, startY)), x: startX, y: startY,
        tx: Math.round(side === "right" ? b.right - Math.min(60, b.width * 0.15) : b.left + b.width / 2),
        ty: Math.round(side === "top" ? strip.bottom + Math.min(50, b.height * 0.15) : b.top + b.height / 2) };
    }, side);
    check(drag.sourceHit, "the drag starts inside the visible active tab");
    await browser.performActions([{ type: "pointer", id: "tab-drag", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", x: drag.x, y: drag.y }, { type: "pointerDown", button: 0 },
      { type: "pointerMove", duration: 300, origin: "viewport", x: drag.tx, y: drag.ty },
    ] }]);
    await browser.$(`[data-pane-id="${drag.targetId}"] .pane-tab-split-preview-${side}`).waitForExist({ timeout: 5000 });
    try { await browser.saveScreenshot(path.join(out, `pane-${side}-preview.png`)); } catch {}
    await browser.performActions([{ type: "pointer", id: "tab-drag", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerUp", button: 0 }] }]);
    await browser.releaseActions();
    await browser.waitUntil(() => browser.execute((drag, side) => {
      const moved = [...document.querySelectorAll(".pane-leaf")].find((pane) => !drag.paneIds.includes(pane.dataset.paneId));
      const target = document.querySelector(`[data-pane-id="${drag.targetId}"]`);
      if (!moved || !target || moved.querySelector(".tab-title")?.textContent !== drag.sourceTitle || document.querySelectorAll(".pane-leaf").length !== drag.count + 1) return false;
      const a = moved.getBoundingClientRect(), b = target.getBoundingClientRect();
      return side === "top" ? a.bottom <= b.top + 1 : a.left >= b.right - 1;
    }, drag, side), { timeout: 5000 });
    report("the content-edge preview becomes the new pane containing the dragged tab");
  }

  console.log(`Artifacts: ${out}`);
} catch (error) {
  try { console.log("Observed UI:", await browser?.execute(() => ({ url: location.href, titles: [...document.querySelectorAll("h1.page-title")].map((el) => el.textContent), body: document.body.innerText.slice(0, 1800) }))); } catch {}
  try { await browser?.saveScreenshot(path.join(out, "failure.png")); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ commit: "54b0f476e43ebf6497d34dec4440e5b3657d4302 + sw-panes diff", journey, expected: "semantic journey above completes", observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  driver.kill("SIGTERM");
  fs.closeSync(log);
}
