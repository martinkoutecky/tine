// GH #704 v2 (D-20261019-VT1): the sidebar Tabs list organizes open tabs as a
// tree. Real WebKitGTK proof: drag rows to nest two tabs (pointer drag through
// the real hit test and layout), collapse the group, restart the app, and the
// tree and its collapse state come back from the session file; the context
// menu's "Move out of group" lifts a tab; then in a window narrower than 640 px
// (where the sidebar list is the only tab surface) the restored tree is there
// and a drag left moves a tab out, again persisted. The top strip stays a flat
// list in tree order throughout.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/sidebar-tabs-tree/native");
const RUN = path.join(ARTIFACT, `run-${Date.now()}`);
const GRAPH = path.join(RUN, "graph");
const XDG_DATA = path.join(RUN, "data");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(RUN, dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const PAGES = ["Tree Alpha", "Tree Beta", "Tree Gamma"];
for (const name of PAGES) fs.writeFileSync(path.join(GRAPH, `pages/${name}.md`), `- ${name} body\n`);
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: XDG_DATA, XDG_CONFIG_HOME: path.join(RUN, "config"), XDG_CACHE_HOME: path.join(RUN, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4592);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4593);

/** The device-local session file(s) under the isolated XDG data dir. */
function sessionTexts(dir = XDG_DATA, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) sessionTexts(full, out);
    else if (entry.name.endsWith(".json")) {
      try { const text = fs.readFileSync(full, "utf8"); if (text.includes("Tree Gamma")) out.push(text); } catch {}
    }
  }
  return out;
}
/** Wait until the saved session's main-pane tabs have exactly `want` shape. */
async function waitForSavedShape(want, what) {
  const deadline = Date.now() + 15_000;
  let seen = null;
  while (Date.now() < deadline) {
    for (const text of sessionTexts()) {
      try {
        const s = JSON.parse(text);
        const tabs = s.layout?.kind === "pane" ? s.layout.tabs : s.tabs;
        seen = (tabs ?? []).map((t) => ({ name: t.history?.[t.pos]?.name ?? t.history?.[t.pos]?.kind, parent: t.parent ?? null, collapsed: !!t.collapsed }));
        if (JSON.stringify(seen) === JSON.stringify(want)) return;
      } catch {}
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`${what}: session file never reached ${JSON.stringify(want)}; last ${JSON.stringify(seen)}`);
}

async function withApp(index, fn) {
  const port = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const target = await startWebdriverApplication(APP, env, nativePort, `run-${index}`);
  const log = fs.openSync(path.join(RUN, `driver-${index}.log`), "w");
  const td = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), {
    env: target.env, stdio: ["ignore", log, log], detached: true,
  });
  let browser;
  try {
    await sleep(2500);
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: tauriCapabilities(APP, `run-${index}`, process.platform, target.debuggerAddress) });
    await ensureMainWindow(browser);
    await browser.setWindowSize(1280, 900);
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
    await fn(browser);
  } catch (error) {
    if (browser) {
      await browser.saveScreenshot(path.join(RUN, `failure-${index}.png`)).catch(() => {});
      const state = await browser.execute(() => ({
        rows: [...document.querySelectorAll("#sidebar-tabs-list .nav-tab")].map((r) => ({ title: r.querySelector(".nav-tab-title")?.textContent, depth: r.dataset.tabDepth })),
        strip: [...document.querySelectorAll(".tab[data-tab-id] .tab-title")].map((n) => n.textContent),
        width: innerWidth,
      })).catch(() => null);
      fs.writeFileSync(path.join(RUN, `failure-${index}.json`), JSON.stringify({ journey: "sidebar-tabs-tree", run: index, error: String(error), state }, null, 2));
    }
    throw error;
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    stopWebdriverApplication(target);
    fs.closeSync(log);
  }
}

/** Open `name` in a background tab through Ctrl+K (middle-button on the row). */
async function openInBackground(browser, name) {
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5000 });
  await input.setValue(name);
  await browser.waitUntil(() => browser.execute((name) => [...document.querySelectorAll(".switcher-row")].some((row) =>
    row.querySelector(".switcher-kind")?.textContent?.trim() === "page" && row.querySelector(".switcher-name")?.textContent?.trim() === name), name), { timeout: 10_000 });
  await browser.execute((name) => {
    const row = [...document.querySelectorAll(".switcher-row")].find((row) => row.querySelector(".switcher-kind")?.textContent?.trim() === "page"
      && row.querySelector(".switcher-name")?.textContent?.trim() === name);
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 1 }));
  }, name);
  await browser.keys("Escape");
  await browser.$(".switcher-input").waitForExist({ reverse: true, timeout: 5000 });
}

const sidebarRows = (browser) => browser.execute(() => [...document.querySelectorAll("#sidebar-tabs-list .nav-tab")].map((r) => ({
  title: r.querySelector(".nav-tab-title")?.textContent?.trim() ?? "",
  depth: Number(r.dataset.tabDepth),
  expanded: r.querySelector(".nav-tab-toggle")?.getAttribute("aria-expanded") ?? null,
})));
const stripTitles = (browser) => browser.execute(() => [...document.querySelectorAll(".tab[data-tab-id] .tab-title")].map((n) => n.textContent.trim()));
async function expectRows(browser, want, what) {
  try {
    await browser.waitUntil(async () => JSON.stringify((await sidebarRows(browser)).map(({ title, depth }) => `${"-".repeat(depth)}${title}`)) === JSON.stringify(want), { timeout: 5000 });
  } catch {
    throw new Error(`${what}: sidebar rows ${JSON.stringify(await sidebarRows(browser))}, wanted ${JSON.stringify(want)}`);
  }
}

/** A real pointer drag of the row titled `title` (grabbed at its title), to the
 *  upper/lower half of the row titled `onto`, `dx` px right of the grab. */
async function dragRow(browser, title, onto, half, dx) {
  const geometry = await browser.execute((title, onto) => {
    const row = (t) => [...document.querySelectorAll("#sidebar-tabs-list .nav-tab")].find((r) => r.querySelector(".nav-tab-title")?.textContent?.trim() === t);
    const grab = row(title)?.querySelector(".nav-tab-title")?.getBoundingClientRect();
    const drop = row(onto)?.getBoundingClientRect();
    return grab && drop ? { x: Math.round(grab.left + 8), y: Math.round(grab.top + grab.height / 2), top: drop.top, height: drop.height } : null;
  }, title, onto);
  if (!geometry) throw new Error(`drag ${title} -> ${onto}: row not found`);
  const endY = Math.round(geometry.top + (half === "upper" ? geometry.height * 0.25 : geometry.height * 0.75));
  await browser.performActions([{
    type: "pointer", id: "sidebar-tabs-drag", parameters: { pointerType: "mouse" },
    actions: [
      { type: "pointerMove", duration: 0, x: geometry.x, y: geometry.y },
      { type: "pointerDown", button: 0 },
      { type: "pointerMove", duration: 60, x: geometry.x + Math.sign(dx || 1) * 6, y: geometry.y + 2 },
      { type: "pointerMove", duration: 120, x: geometry.x + dx, y: endY },
      { type: "pointerUp", button: 0 },
    ],
  }]);
  await browser.releaseActions();
}

async function ensureLeftSidebar(browser) {
  if (await browser.$('[data-sidebar-section="tabs"]').isExisting()) return;
  await browser.execute(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await browser.keys(["t", "l"]);
  await browser.$('[data-sidebar-section="tabs"]').waitForExist({ timeout: 5000 });
}

// Run 0: build the tree with real drags, collapse it.
await withApp(0, async (browser) => {
  for (const name of PAGES) await openInBackground(browser, name);
  await ensureLeftSidebar(browser);
  await expectRows(browser, ["Journals", ...PAGES], "four open tabs");
  await dragRow(browser, "Tree Beta", "Tree Beta", "lower", 24); // one step right: under Alpha
  await expectRows(browser, ["Journals", "Tree Alpha", "-Tree Beta", "Tree Gamma"], "Beta nested under Alpha");
  await dragRow(browser, "Tree Gamma", "Tree Gamma", "lower", 40); // two steps right: under Beta
  await expectRows(browser, ["Journals", "Tree Alpha", "-Tree Beta", "--Tree Gamma"], "Gamma nested under Beta");
  const strip = await stripTitles(browser);
  if (JSON.stringify(strip.slice(1)) !== JSON.stringify(PAGES)) throw new Error(`strip is not the flat tree order: ${JSON.stringify(strip)}`);
  await browser.execute(() => [...document.querySelectorAll("#sidebar-tabs-list .nav-tab")]
    .find((r) => r.querySelector(".nav-tab-title")?.textContent?.trim() === "Tree Alpha")?.querySelector(".nav-tab-toggle")?.click());
  await expectRows(browser, ["Journals", "Tree Alpha"], "collapsed Alpha hides its group in the sidebar");
  if ((await stripTitles(browser)).length !== 4) throw new Error("collapse changed the top strip");
  await waitForSavedShape([
    { name: "journals", parent: null, collapsed: false },
    { name: "Tree Alpha", parent: null, collapsed: true },
    { name: "Tree Beta", parent: 1, collapsed: false },
    { name: "Tree Gamma", parent: 2, collapsed: false },
  ], "after nesting and collapse");
  await browser.saveScreenshot(path.join(RUN, "nested-collapsed.png"));
  console.log("PASS run 0: drag-nested two tabs, collapsed, session saved");
});

// Run 1: a fresh process restores the tree and its collapse state.
await withApp(1, async (browser) => {
  await ensureLeftSidebar(browser);
  await expectRows(browser, ["Journals", "Tree Alpha"], "restart restores the collapsed group");
  const alpha = (await sidebarRows(browser)).find((r) => r.title === "Tree Alpha");
  if (alpha?.expanded !== "false") throw new Error(`restored Alpha is not collapsed: ${JSON.stringify(alpha)}`);
  await browser.execute(() => [...document.querySelectorAll("#sidebar-tabs-list .nav-tab")]
    .find((r) => r.querySelector(".nav-tab-title")?.textContent?.trim() === "Tree Alpha")?.querySelector(".nav-tab-toggle")?.click());
  await expectRows(browser, ["Journals", "Tree Alpha", "-Tree Beta", "--Tree Gamma"], "restart restores the nesting");
  // The non-drag path: the nested row's context menu.
  const gamma = await browser.execute(() => {
    const r = [...document.querySelectorAll("#sidebar-tabs-list .nav-tab")].find((r) => r.querySelector(".nav-tab-title")?.textContent?.trim() === "Tree Gamma");
    const b = r.querySelector(".nav-tab-title").getBoundingClientRect();
    return { x: Math.round(b.left + 8), y: Math.round(b.top + b.height / 2) };
  });
  await browser.performActions([{ type: "pointer", id: "sidebar-tabs-menu", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, x: gamma.x, y: gamma.y }, { type: "pointerDown", button: 2 }, { type: "pointerUp", button: 2 },
  ] }]);
  await browser.releaseActions();
  const item = await browser.$("//*[contains(@class,'ctx-item') and normalize-space()='Move out of group']");
  await item.waitForExist({ timeout: 5000 });
  await item.click();
  await expectRows(browser, ["Journals", "Tree Alpha", "-Tree Beta", "-Tree Gamma"], "Move out of group");
  await waitForSavedShape([
    { name: "journals", parent: null, collapsed: false },
    { name: "Tree Alpha", parent: null, collapsed: false },
    { name: "Tree Beta", parent: 1, collapsed: false },
    { name: "Tree Gamma", parent: 1, collapsed: false },
  ], "after Move out of group");
  console.log("PASS run 1: tree and collapse restored after restart; Move out of group");
});

// Run 2: a window narrower than 640 px, where the sidebar list is the only tab surface.
await withApp(2, async (browser) => {
  await browser.setWindowSize(600, 900);
  await browser.waitUntil(() => browser.execute(() => innerWidth < 640), { timeout: 10_000, timeoutMsg: "window did not narrow below 640 px" });
  await browser.waitUntil(async () => (await stripTitles(browser)).length === 0, { timeout: 5000, timeoutMsg: "the top strip still shows below 640 px" });
  await ensureLeftSidebar(browser);
  await expectRows(browser, ["Journals", "Tree Alpha", "-Tree Beta", "-Tree Gamma"], "narrow window shows the restored tree");
  await dragRow(browser, "Tree Gamma", "Tree Gamma", "lower", -24);
  await expectRows(browser, ["Journals", "Tree Alpha", "-Tree Beta", "Tree Gamma"], "drag left moves Gamma out");
  await waitForSavedShape([
    { name: "journals", parent: null, collapsed: false },
    { name: "Tree Alpha", parent: null, collapsed: false },
    { name: "Tree Beta", parent: 1, collapsed: false },
    { name: "Tree Gamma", parent: null, collapsed: false },
  ], "after the narrow-window drag");
  await browser.saveScreenshot(path.join(RUN, "narrow.png"));
  console.log("PASS run 2: narrow window tree, drag left persisted");
});
console.log(`PASS: sidebar tab tree (artifacts ${RUN})`);
