// Real-app acceptance for the mobile touch gestures (GH #501, #492): the left
// edge swipe (drawer pull and iOS Back), and the block-row swipe (indent,
// outdent, action menu). It drives the production binary under WebKitGTK and
// dispatches SYNTHETIC touch sequences into the page, so it proves the JS
// recognizers, the app wiring (router/drawer/outline commands) and the on-disk
// result. It does NOT prove that WKWebView / Android WebView deliver touch
// events from the physical screen edge, scroll-versus-swipe arbitration by the
// OS compositor, or the feel of the finger-following animation; only a device
// can. The binary needs the harness hook TINE_E2E_TOUCH_GESTURES (an E2E-only
// env, src-tauri/src/lib.rs) because desktop Linux otherwise installs no touch
// code at all; the hook never changes the reported platform.
//
// D-20261019-FT1 adds the sidebar Favorites touch hold: the rows take the
// Tabs list's gesture (hold still, then drag, to reorder; a finger that moves
// first scrolls; a tap opens). That gesture is driven by Pointer Events with
// pointerType "touch" (the recognizer is pointer-based, not touch-event
// based), dispatched here as synthetic PointerEvents through the page's real
// hit test and layout; the result is read from config.edn on disk.
//
// GH #921: the same still hold also opens the row's own menu (the one a
// right-click opens), and moving the held finger closes it and drags. The
// journey holds a favorite, keeps the menu through the release, and uses its
// Open in new tab, as the reporter does.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(process.env.CARGO_TARGET_DIR || path.join(ROOT, "target"), "release/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync(path.join(process.env.TMPDIR || "/tmp", "tine-touch-gestures-e2e-"));
const GRAPH = path.join(TMP, "graph");
const PAGE = path.join(GRAPH, "pages", "Swipe target.md");
const CONFIG = path.join(GRAPH, "logseq", "config.edn");
const FAVORITES = ["Fav Alpha", "Fav Beta", "Fav Gamma"];
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts"));
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4592);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4593);
const MOTION_ONLY = process.env.E2E_DRAWER_MOTION_ONLY === "1";
const PARENT = "swipe-parent-501";
const NESTED = "swipe-nested-501";
const CHILD = "swipe-child-501";
const MENU = "swipe-menu-501";
const PAGE_TEXT = [
  "- Parent A",
  `  id:: ${PARENT}`,
  "\t- Nested N",
  `\t  id:: ${NESTED}`,
  "- Child B",
  `  id:: ${CHILD}`,
  "- Menu M",
  `  id:: ${MENU}`,
  "",
].join("\n");

for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state", "reduced-config/gtk-3.0"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(path.join(TMP, "home"), { recursive: true });
fs.writeFileSync(path.join(TMP, "xdg", "reduced-config", "gtk-3.0", "settings.ini"), "[Settings]\ngtk-enable-animations=false\n");
fs.mkdirSync(ARTIFACT, { recursive: true });
fs.writeFileSync(CONFIG, `{:favorites [${FAVORITES.map((name) => JSON.stringify(name)).join(" ")}]}\n`);
for (const name of FAVORITES) fs.writeFileSync(path.join(GRAPH, "pages", `${name}.md`), `- ${name} body\n`);
fs.writeFileSync(PAGE, PAGE_TEXT);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), "- Touch gesture home\n");

const proof = { scenario: "gh-501-492-native-touch-gestures", app: APP, testedCommit: spawnSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).stdout.trim(), sourceDelta: "og-mobile-sidebar-slide worktree diff", startedAt: new Date().toISOString(), touchEvents: null, motion: [], steps: [] };

function assert(condition, message, detail) {
  if (!condition) throw new Error(`${message}${detail === undefined ? "" : `: ${JSON.stringify(detail)}`}`);
}
const note = (step) => { proof.steps.push(step); console.log(`ok  ${step}`); };

function baseEnv(platform, reduced) {
  return {
    ...process.env,
    TINE_GRAPH: GRAPH,
    HOME: path.join(TMP, "home"),
    XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
    XDG_CONFIG_HOME: path.join(TMP, "xdg", reduced ? "reduced-config" : "config"),
    XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
    XDG_STATE_HOME: path.join(TMP, "xdg", "state"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    LIBGL_ALWAYS_SOFTWARE: "1",
    GDK_BACKEND: "x11",
    TINE_E2E_FORCE_MOBILE_DRAWERS: "1",
    TINE_E2E_TOUCH_GESTURES: platform,
  };
}

async function withApp(index, platform, fn, reduced = false) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const log = fs.openSync(path.join(ARTIFACT, `driver-${index}-${platform}.log`), "w");
  const driver = spawn(DRIVER, [
    "--port", String(driverPort),
    "--native-port", String(nativePort),
    "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
  ], { env: baseEnv(platform, reduced), detached: true, stdio: ["ignore", log, log] });
  let browser;
  try {
    await sleep(2500);
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
      connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await browser.$(".app-container").waitForExist({ timeout: 20_000 });
    await browser.$(".main-content").waitForExist({ timeout: 20_000 });
    await fn(browser);
    await sleep(300);
  } catch (error) {
    const screenshot = path.join(ARTIFACT, `failure-${index}-${platform}.png`);
    try { await browser?.saveScreenshot(screenshot); } catch {}
    fs.writeFileSync(path.join(ARTIFACT, `failure-${index}-${platform}.json`), JSON.stringify({ testedCommit: proof.testedCommit, journey: `touch gestures / ${platform} / reduced=${reduced}`, expected: "gesture and drawer motion assertions pass", observed: String(error), screenshot, classification: "ambiguous; inspect assertion and driver log" }, null, 2));
    throw error;
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    try { fs.closeSync(log); } catch {}
  }
}

// One touch event into the page. A real TouchEvent when WebKitGTK offers one,
// otherwise a plain Event carrying the same touches/changedTouches shape; which
// one ran is recorded in the proof. The target is chosen at touchstart (by
// selector, else the element under the point) and kept for the whole gesture,
// as a browser does.
const FIRE = (type, x, y, selector) => {
  const g = window;
  if (type === "touchstart") g.__e2eTouchTarget = selector ? document.querySelector(selector) : document.elementFromPoint(x, y);
  const target = g.__e2eTouchTarget;
  if (!target) return { error: `no target for ${selector ?? `${x},${y}`}` };
  const init = { identifier: 1, target, clientX: x, clientY: y, pageX: x, pageY: y, screenX: x, screenY: y };
  let touch;
  try { touch = new Touch(init); } catch { touch = init; }
  const live = type === "touchend" || type === "touchcancel" ? [] : [touch];
  let event;
  let real = true;
  try {
    event = new TouchEvent(type, { bubbles: true, cancelable: true, composed: true, touches: live, targetTouches: live, changedTouches: [touch] });
  } catch {
    real = false;
    event = new Event(type, { bubbles: true, cancelable: true, composed: true });
    for (const [key, value] of Object.entries({ touches: live, targetTouches: live, changedTouches: [touch] })) {
      Object.defineProperty(event, key, { value });
    }
  }
  target.dispatchEvent(event);
  return { real, prevented: event.defaultPrevented };
};

async function touch(browser, type, x, y, selector) {
  const result = await browser.execute(FIRE, type, Math.round(x), Math.round(y), selector ?? null);
  assert(!result.error, "touch dispatch failed", result);
  if (proof.touchEvents === null) proof.touchEvents = result.real ? "real TouchEvent" : "fallback Event with touches";
  return result;
}

/** A whole gesture: down at (x,y), moves to each offset, up at the last one. */
async function swipe(browser, { x, y, path: offsets, selector }) {
  await touch(browser, "touchstart", x, y, selector);
  let last = { x, y };
  for (const [dx, dy] of offsets) {
    last = { x: x + dx, y: y + dy };
    await touch(browser, "touchmove", last.x, last.y);
    await sleep(25);
  }
  await touch(browser, "touchend", last.x, last.y);
}
const steps = (dx, dy = 0, n = 6) => Array.from({ length: n }, (_, i) => [Math.round((dx * (i + 1)) / n), Math.round((dy * (i + 1)) / n)]);

const activeDrawer = (browser) => browser.execute(() => document.querySelector(".app-container")?.getAttribute("data-active-drawer") || null);
const readPage = () => fs.readFileSync(PAGE, "utf8");

async function expectNoDrawerFor(browser, ms, what) {
  await sleep(ms);
  const drawer = await activeDrawer(browser);
  assert(drawer === null, `${what}: a drawer opened`, drawer);
}

async function waitDrawer(browser, side) {
  await browser.waitUntil(async () => (await activeDrawer(browser)) === side, { timeout: 8_000, timeoutMsg: `${side} drawer did not open` });
  // Geometry-based hold/drag journeys need the row at its resting position.
  // Visibility state intentionally changes before the slide has completed.
  await browser.waitUntil(() => browser.execute((side) => {
    const panel = document.querySelector(`[data-mobile-drawer-panel="${side}"]`);
    return !!panel && panel.getAnimations().length === 0 && (getComputedStyle(panel).transform === "none" || new DOMMatrixReadOnly(getComputedStyle(panel).transform).m41 === 0);
  }, side), { timeout: 4_000, timeoutMsg: `${side} drawer did not settle before measuring rows` });
}
async function waitNoDrawer(browser) {
  await browser.waitUntil(async () => (await activeDrawer(browser)) === null, { timeout: 8_000, timeoutMsg: "drawer did not close" });
}
// A fresh forced-phone launch may start with the left drawer showing (the
// existing mobile-drawers journey closes it the same way); the gestures under
// test start from a closed drawer.
async function closeStartupDrawer(browser) {
  if ((await activeDrawer(browser)) !== null) {
    await browser.$(".mobile-drawer-close, .rs-close").click();
    await waitNoDrawer(browser);
  }
}
const blockSel = (id) => `[data-block-ref='${id}'] .block-content`;


// ---- Favorites touch hold (D-20261019-FT1) ---------------------------------
const FAV_ROWS = "#sidebar-favorites-list .nav-page";
/** The favorites in the order config.edn lists them (what Logseq reads). */
const savedFavorites = () => {
  const m = /:favorites\s*\[([^\]]*)\]/.exec(fs.readFileSync(CONFIG, "utf8"));
  return m ? [...m[1].matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((x) => x[1].toLowerCase()) : [];
};
const lowered = (names) => names.map((n) => n.toLowerCase());
const shownFavorites = (browser) => browser.execute((sel) =>
  [...document.querySelectorAll(sel)].map((r) => r.querySelector(".nav-page-label, .sidebar-title, [class*='title']")?.textContent?.trim() ?? r.textContent.trim()), FAV_ROWS);
const favRect = (browser, title) => browser.execute((sel, title) => {
  const row = [...document.querySelectorAll(sel)].find((r) => r.textContent.trim().includes(title));
  if (!row) return null;
  const r = row.getBoundingClientRect();
  return { x: Math.round(r.left + Math.min(40, r.width / 2)), top: r.top, height: r.height, mid: Math.round(r.top + r.height / 2) };
}, FAV_ROWS, title);
/** One touch-type Pointer Event: the press goes to the element under the point
 *  (as the browser targets it), later events to the document (as the recognizer listens). */
async function touchPointer(browser, type, x, y) {
  const r = await browser.execute((type, x, y) => {
    const init = { bubbles: true, cancelable: true, composed: true, pointerId: 41, pointerType: "touch", isPrimary: true, clientX: x, clientY: y, button: 0, buttons: type === "pointerup" ? 0 : 1 };
    try {
      (type === "pointerdown" ? document.elementFromPoint(x, y) : document).dispatchEvent(new PointerEvent(type, init));
      return { ok: true };
    } catch (e) { return { ok: false, error: String(e) }; }
  }, type, x, y);
  assert(r.ok, "touch pointer dispatch failed", r);
}
const menuOpen = (browser) => browser.execute(() => Boolean(document.querySelector(".ctx-menu")));
const openTabCount = (browser) => browser.execute(() =>
  Number(document.querySelector("[data-sidebar-section='tabs'] .nav-section-count")?.textContent ?? 1));
async function fingerMove(browser, from, to, steps = 6) {
  for (let i = 1; i <= steps; i++) {
    await touchPointer(browser, "pointermove", from.x + ((to.x - from.x) * i) / steps, from.y + ((to.y - from.y) * i) / steps);
    await sleep(25);
  }
}

// ---- iOS: edge swipe (drawer pull + Back) and block swipes ------------------
if (!MOTION_ONLY) await withApp(0, "ios", async (browser) => {
  const vp = await browser.execute(() => ({ w: innerWidth, h: innerHeight, mode: document.documentElement.getAttribute("data-touch-gestures") }));
  assert(vp.w < 640 && vp.mode === "ios", "journey did not start as a phone-width iOS-gesture window", vp);
  const Y = Math.round(vp.h / 2);

  await closeStartupDrawer(browser);
  // Item 2. Fresh launch: nothing to go back to, so the left edge pulls the drawer.
  await swipe(browser, { x: 10, y: Y, path: steps(30) });                  // short of the open threshold
  await expectNoDrawerFor(browser, 400, "a 30px edge swipe");
  await swipe(browser, { x: 10, y: Y, path: steps(12, 90) });              // vertical-dominant: a scroll
  await expectNoDrawerFor(browser, 400, "a vertical edge start");
  await swipe(browser, { x: 100, y: Y, path: steps(120) });                // not from the edge
  await expectNoDrawerFor(browser, 400, "a swipe that did not start at the edge");
  await swipe(browser, { x: 10, y: Y, path: steps(120) });
  await waitDrawer(browser, "left");
  note("left edge swipe opens the left drawer; short, vertical and mid-screen swipes do not");

  // Item 3, Back ladder: with the drawer open Back has something to pop, so the
  // same edge swipe is now Back, and its first rung is the drawer.
  await swipe(browser, { x: 10, y: Y, path: steps(200) });
  await waitNoDrawer(browser);
  note("iOS edge swipe with the drawer open runs Back and closes the drawer");

  // Item 3, router history.
  await openPageByName(browser, "Swipe target");
  assert((await currentPageTitle(browser)) === "Swipe target", "page did not open");
  await swipe(browser, { x: 10, y: Y, path: steps(30) });                  // snap back
  await sleep(500);
  const after = await browser.execute(() => ({
    title: document.querySelector("h1.page-title")?.textContent?.trim() ?? null,
    transform: document.querySelector(".app-container > .main-container")?.style.transform ?? "",
  }));
  assert(after.title === "Swipe target", "a short Back swipe left the page", after);
  assert(after.transform === "" || after.transform === "translateX(0px)", "the page did not snap back", after);
  await expectNoDrawerFor(browser, 100, "a Back-mode edge swipe");
  // GH #501: a committed Back slides the page fully off to the right before the
  // page changes, like the native iOS back swipe. Record every transform the
  // page container takes, with the title at that moment (no timing guess).
  await browser.execute(() => {
    const el = document.querySelector(".app-container > .main-container");
    const w = window;
    w.__slideLog = [];
    w.__slideObs = new MutationObserver(() => w.__slideLog.push({
      transform: el.style.transform,
      title: document.querySelector("h1.page-title")?.textContent?.trim() ?? null,
    }));
    w.__slideObs.observe(el, { attributes: true, attributeFilter: ["style"] });
  });
  await swipe(browser, { x: 10, y: Y, path: steps(220) });
  await browser.waitUntil(async () => (await currentPageTitle(browser)) !== "Swipe target", { timeout: 8_000, timeoutMsg: "a long edge swipe did not go back" });
  const slide = await browser.execute(() => {
    window.__slideObs.disconnect();
    return { width: innerWidth, log: window.__slideLog };
  });
  const fullOff = slide.log.find((r) => r.transform === `translateX(${slide.width}px)`);
  assert(fullOff && fullOff.title === "Swipe target", "a committed Back did not slide the page fully off before going back (GH #501)", slide);
  await browser.waitUntil(async () => (await browser.execute(() => document.querySelector(".app-container > .main-container")?.style.transform ?? "")) === "", { timeout: 4_000, timeoutMsg: "the page kept its slide-off transform after Back" });
  note("iOS edge swipe goes back through router history; a short one snaps back");

  // Item 1. Block swipes on the routed page.
  await openPageByName(browser, "Swipe target");
  const row = blockSel(CHILD);
  await browser.$(row).waitForExist({ timeout: 10_000 });
  await browser.$(blockSel(NESTED)).waitForExist({ timeout: 10_000 });
  const rect = await browser.execute((sel) => { const r = document.querySelector(sel).getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; }, row);
  const rowY = rect.y + rect.h / 2;
  const rowX = Math.max(60, rect.x + 60);

  // Nothing below the thresholds, and a vertical-dominant drag is a scroll.
  await swipe(browser, { x: rowX, y: rowY, path: steps(20), selector: row });
  await swipe(browser, { x: rowX, y: rowY, path: steps(20, 90), selector: row });
  await swipe(browser, { x: rowX, y: rowY, path: steps(-20), selector: row });
  await sleep(700);
  assert(readPage() === PAGE_TEXT, "a below-threshold or vertical block swipe edited the page", readPage());
  note("short and vertical block swipes change nothing");

  // Right swipe indents Child B under Parent A.
  await swipe(browser, { x: rowX, y: rowY, path: steps(70), selector: row });
  await waitForFileText(PAGE, (text) => /^[ \t]+- Child B/m.test(text) && /^- Parent A/m.test(text));
  note("a right swipe indents the block, on disk");

  // Short left swipe outdents Nested N to the top level.
  const nestedSel = blockSel(NESTED);
  const nrect = await browser.execute((sel) => { const r = document.querySelector(sel).getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; }, nestedSel);
  await swipe(browser, { x: Math.max(80, nrect.x + 80), y: nrect.y + nrect.h / 2, path: steps(-55), selector: nestedSel });
  await waitForFileText(PAGE, (text) => /^- Nested N/m.test(text));
  note("a short left swipe outdents the block, on disk");

  // Long left swipe opens the action menu on the selected block.
  const menuSel = blockSel(MENU);
  const mrect = await browser.execute((sel) => { const r = document.querySelector(sel).getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; }, menuSel);
  const before = readPage();
  await swipe(browser, { x: Math.max(200, mrect.x + 200), y: mrect.y + mrect.h / 2, path: steps(-180), selector: menuSel });
  await browser.$(".ctx-menu").waitForExist({ timeout: 5_000 });
  const selected = await browser.execute((id) => {
    const el = document.querySelector(`[data-block-ref='${id}']`);
    return Boolean(el && (el.classList.contains("selected") || el.querySelector(".block-main")?.classList.contains("selected")));
  }, MENU);
  assert(selected, "the long left swipe did not select its block");
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".ctx-menu")), { timeout: 5_000, timeoutMsg: "menu did not close" });
  assert(readPage() === before, "opening the action menu edited the page");
  note("a long left swipe selects the block and opens its action menu");
  proof.finalPage = readPage();
});

// ---- Android: the OS owns Back, so the same edge swipe is only the drawer ----
if (!MOTION_ONLY) await withApp(1, "android", async (browser) => {
  const vp = await browser.execute(() => ({ h: innerHeight, mode: document.documentElement.getAttribute("data-touch-gestures") }));
  assert(vp.mode === "android", "journey did not start with Android gestures", vp);
  const Y = Math.round(vp.h / 2);
  await openPageByName(browser, "Swipe target");                           // Back would now have somewhere to go
  await closeStartupDrawer(browser);
  assert((await activeDrawer(browser)) === null, "drawer still open before the Android edge swipe");
  await swipe(browser, { x: 10, y: Y, path: steps(120) });
  await waitDrawer(browser, "left");
  assert((await currentPageTitle(browser)) === "Swipe target", "the Android edge swipe navigated instead of opening the drawer");
  note("on Android the edge swipe opens the drawer even when history exists (Back is the OS gesture)");

  // D-20261019-FT1: Favorites rows take the Tabs touch hold.
  await browser.$(FAV_ROWS).waitForExist({ timeout: 10_000 });
  assert(JSON.stringify(savedFavorites()) === JSON.stringify(lowered(FAVORITES)), "config.edn did not start in the seeded order", savedFavorites());
  const alpha = await favRect(browser, "Fav Alpha");
  const gamma = await favRect(browser, "Fav Gamma");
  assert(alpha && gamma, "favorite rows not laid out", { alpha, gamma });
  const start = { x: alpha.x, y: alpha.mid };

  // A finger that moves at once is a scroll: nothing is picked up, nothing written.
  await touchPointer(browser, "pointerdown", start.x, start.y);
  await fingerMove(browser, start, { x: start.x, y: gamma.mid }, 4);
  await sleep(800); // longer than the hold: a hold that was not cancelled would pick the row up now
  await touchPointer(browser, "pointerup", start.x, gamma.mid);
  await sleep(500);
  assert(JSON.stringify(savedFavorites()) === JSON.stringify(lowered(FAVORITES)), "a finger that moved at once reordered the favorites", savedFavorites());
  assert(!(await browser.execute(() => document.documentElement.classList.contains("drag-selection-suppressed"))), "a scroll left the drag active");
  assert(!(await menuOpen(browser)), "a finger that moved at once opened a menu");
  note("a finger that moves first on a Favorites row scrolls: no reorder, no menu, nothing written");

  // A still hold, then a drag to the lower half of the last row, reorders and persists.
  await touchPointer(browser, "pointerdown", start.x, start.y);
  await sleep(800);
  assert(await browser.execute(() => document.documentElement.classList.contains("drag-selection-suppressed")), "the hold did not pick the row up");
  assert(await menuOpen(browser), "the still hold did not open the row's menu (GH #921)");
  await fingerMove(browser, start, { x: start.x, y: Math.round(gamma.top + gamma.height * 0.75) });
  assert(!(await menuOpen(browser)), "dragging the held row left its menu open");
  await touchPointer(browser, "pointerup", start.x, Math.round(gamma.top + gamma.height * 0.75));
  const want = ["Fav Beta", "Fav Gamma", "Fav Alpha"];
  await waitForFileText(CONFIG, () => JSON.stringify(savedFavorites()) === JSON.stringify(lowered(want)));
  await browser.waitUntil(async () => JSON.stringify((await shownFavorites(browser)).map((t) => t.replace(/^\W+/, ""))) === JSON.stringify(want), { timeout: 8_000, timeoutMsg: `sidebar Favorites order did not follow: ${JSON.stringify(await shownFavorites(browser))}` });
  assert(!(await menuOpen(browser)), "a menu opened after the drag");
  note("a still hold then a drag closes the hold's menu, reorders Favorites and writes config.edn :favorites in that order");

  // GH #921: a still hold opens the favorite's page menu, the release keeps it
  // (its compatibility click included), and Open in new tab opens a tab.
  const held = await favRect(browser, "Fav Gamma");
  const tabsBefore = await openTabCount(browser);
  await touchPointer(browser, "pointerdown", held.x, held.mid);
  await sleep(800);
  await touchPointer(browser, "pointerup", held.x, held.mid);
  await browser.execute((x, y) => document.elementFromPoint(x, y)?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1, clientX: x, clientY: y })), held.x, held.mid);
  await sleep(300);
  const menu = await browser.execute(() => ({
    open: Boolean(document.querySelector(".ctx-menu[role='menu']")),
    items: [...document.querySelectorAll(".ctx-menu [role='menuitem']")].map((el) => el.textContent.trim()),
  }));
  assert(menu.open, "a still hold on a Favorites row did not leave its page menu open after the release (GH #921)", menu);
  assert(JSON.stringify(savedFavorites()) === JSON.stringify(lowered(want)), "the hold that opened the menu reordered the favorites", savedFavorites());
  const item = await browser.execute(() => {
    const el = [...document.querySelectorAll(".ctx-menu [role='menuitem']")].find((e) => e.textContent.trim() === "Open in new tab");
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
  });
  assert(item, "the favorite's hold menu has no Open in new tab", menu.items);
  await touchPointer(browser, "pointerdown", item.x, item.y);
  await sleep(80);
  await touchPointer(browser, "pointerup", item.x, item.y);
  await browser.execute((x, y) => document.elementFromPoint(x, y)?.click(), item.x, item.y);
  await browser.waitUntil(async () => !(await menuOpen(browser)) && (await openTabCount(browser)) === tabsBefore + 1, { timeout: 8_000, timeoutMsg: `Open in new tab from the hold menu did not add a tab: ${tabsBefore} -> ${await openTabCount(browser)}` });
  note("a still hold on a Favorites row opens its page menu, the release keeps it, and its Open in new tab adds a tab (GH #921)");
  if ((await activeDrawer(browser)) !== "left") {
    await swipe(browser, { x: 10, y: Y, path: steps(120) });
    await waitDrawer(browser, "left");
  }

  // A tap still opens the favorite (the click a tap produces, after a touch press and release).
  const beta = await favRect(browser, "Fav Beta");
  await touchPointer(browser, "pointerdown", beta.x, beta.mid);
  await sleep(80);
  await touchPointer(browser, "pointerup", beta.x, beta.mid);
  await browser.execute((sel) => [...document.querySelectorAll(sel)].find((r) => r.textContent.includes("Fav Beta")).click(), FAV_ROWS);
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Fav Beta", { timeout: 8_000, timeoutMsg: "a tap did not open the favorite" });
  assert(JSON.stringify(savedFavorites()) === JSON.stringify(lowered(want)), "a tap changed the favorites order", savedFavorites());
  note("a tap on a Favorites row opens the favorite and reorders nothing");
});

// GH #501: actual WebKit compositor samples, on a routed page, for both edges.
// OG timing/easing is covered cheaply; this journey proves a visible position
// between endpoints, a fading scrim, immediate isolation and eventual disposal.
const drawerToggle = (side) => side === "left" ? ".navigation-actions button:first-child" : '[data-topbar-overflow-action="right-sidebar"]';
async function sampleDrawerMotion(browser, side, closing) {
  if (side === "right" && !closing) await browser.$("[data-topbar-overflow-trigger]").click();
  return browser.executeAsync((side, closing, toggle, done) => {
    const selector = `[data-mobile-drawer-panel="${side}"]`;
    const before = document.querySelector(selector);
    document.querySelector(closing ? "[data-mobile-drawer-scrim]" : toggle).click();
    const panel = document.querySelector(selector) || before;
    const immediate = { active: document.querySelector(".app-container").dataset.activeDrawer, inert: panel?.inert, hidden: panel?.getAttribute("aria-hidden") };
    setTimeout(() => {
      const scrim = document.querySelector("[data-mobile-drawer-scrim]");
      const transform = panel ? getComputedStyle(panel).transform : "none";
      const x = transform === "none" ? 0 : new DOMMatrixReadOnly(transform).m41;
      // Pause for a mid-slide screenshot; resume immediately after capture.
      for (const el of [panel, scrim]) for (const a of el?.getAnimations() || []) a.pause();
      done({ side, closing, immediate, x, width: panel?.offsetWidth, opacity: scrim ? Number(getComputedStyle(scrim).opacity) : null, transform });
    }, 70);
  }, side, closing, drawerToggle(side));
}
async function drawerMotionJourney(browser, platform) {
  await closeStartupDrawer(browser);
  await openPageByName(browser, "Swipe target");
  for (const side of ["left", "right"]) {
    for (const closing of [false, true]) {
      const sample = await sampleDrawerMotion(browser, side, closing);
      proof.motion.push({ platform, ...sample });
      assert(Math.abs(sample.x) > 0 && Math.abs(sample.x) < sample.width, "drawer did not pass through a mid-slide position", sample);
      assert(sample.opacity > 0 && sample.opacity < 1, "scrim did not fade between endpoints", sample);
      if (closing) assert(sample.immediate.active === "" && sample.immediate.inert && sample.immediate.hidden === "true", "closing drawer remained interactive", sample);
      await browser.saveScreenshot(path.join(ARTIFACT, `${platform}-${side}-${closing ? "out" : "in"}-mid.png`));
      await browser.execute(() => {
        for (const el of document.querySelectorAll("[data-mobile-drawer-panel], [data-mobile-drawer-scrim]")) for (const a of el.getAnimations()) a.play();
      });
      await browser.waitUntil(() => browser.execute((side, closing) => {
        const panel = document.querySelector(`[data-mobile-drawer-panel="${side}"]`);
        return closing ? !panel : !!panel && panel.getAnimations().length === 0;
      }, side, closing), { timeout: 4_000, timeoutMsg: `${side} drawer motion did not settle` });
      await browser.saveScreenshot(path.join(ARTIFACT, `${platform}-${side}-${closing ? "closed" : "open"}.png`));
    }
  }
  // Preload a favorite, return to the routed source, then tap that real page
  // link. The source route must leave while the outgoing drawer is still
  // present; loading the destination's content is independently asynchronous.
  await openPageByName(browser, "Fav Alpha");
  await openPageByName(browser, "Swipe target");
  await browser.$(drawerToggle("left")).click();
  await waitDrawer(browser, "left");
  const navigation = await browser.executeAsync((done) => {
    [...document.querySelectorAll("#sidebar-favorites-list .nav-page")].find(row => row.textContent.includes("Fav Alpha")).click();
    queueMicrotask(() => {
      const panel = document.querySelector('[data-mobile-drawer-panel="left"]');
      done({ title: document.querySelector("h1.page-title")?.textContent.trim(), closing: panel?.dataset.drawerMotion, animations: panel?.getAnimations().length, active: document.querySelector(".app-container").dataset.activeDrawer });
    });
  });
  proof.motion.push({ platform, navigation });
  assert(navigation.title !== "Swipe target" && navigation.closing === "closing" && navigation.animations > 0 && navigation.active === "", "page navigation waited for drawer motion", navigation);
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Fav Alpha", { timeout: 8_000, timeoutMsg: "favorite navigation did not reach its destination" });
  note(`${platform}: both phone drawers slide in/out, scrim fades, departing content is inert and unmounts`);
  note(`${platform}: tapping a favorite navigates while the drawer is still sliding out`);
}
await withApp(2, "ios", browser => drawerMotionJourney(browser, "ios"));
await withApp(3, "android", browser => drawerMotionJourney(browser, "android"));
await withApp(4, "ios", async (browser) => {
  await closeStartupDrawer(browser);
  await openPageByName(browser, "Swipe target");
  assert(await browser.execute(() => matchMedia("(prefers-reduced-motion: reduce)").matches), "GTK Reduce Motion fixture did not reach WebKit");
  for (const side of ["left", "right"]) {
    if (side === "right") await browser.$("[data-topbar-overflow-trigger]").click();
    const sample = await browser.execute((side, toggle) => {
      document.querySelector(toggle).click();
      const panel = document.querySelector(`[data-mobile-drawer-panel="${side}"]`);
      const scrim = document.querySelector("[data-mobile-drawer-scrim]");
      const opened = { exists: !!panel, animations: panel?.getAnimations().length, transform: panel && getComputedStyle(panel).transform, opacity: scrim && getComputedStyle(scrim).opacity, scrimAnimations: scrim?.getAnimations().length };
      document.querySelector(side === "left" ? ".mobile-drawer-close" : ".rs-close").click();
      return { side, opened, closed: !document.querySelector(`[data-mobile-drawer-panel="${side}"]`) && !document.querySelector("[data-mobile-drawer-scrim]") };
    }, side, drawerToggle(side));
    proof.motion.push({ reduced: true, ...sample });
    assert(sample.opened.exists && sample.opened.animations === 0 && sample.opened.scrimAnimations === 0 && sample.opened.transform === "none" && sample.opened.opacity === "1" && sample.closed, "Reduce Motion did not make drawer and shade instant", sample);
  }
  note("system Reduce Motion: both drawers and scrim open/close instantly in WebKit");
}, true);

proof.finishedAt = new Date().toISOString();
proof.result = "pass";
const proofPath = path.join(ARTIFACT, "proof.json");
fs.writeFileSync(proofPath, `${JSON.stringify(proof, null, 2)}\n`);
console.log(`PASS: native touch gestures (${proof.touchEvents}); proof: ${proofPath}`);
