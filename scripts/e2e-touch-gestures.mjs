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
import { spawn } from "node:child_process";
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
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts"));
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4592);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4593);
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
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(ARTIFACT, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(PAGE, PAGE_TEXT);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), "- Touch gesture home\n");

const proof = { scenario: "gh-501-492-native-touch-gestures", app: APP, startedAt: new Date().toISOString(), touchEvents: null, steps: [] };

function assert(condition, message, detail) {
  if (!condition) throw new Error(`${message}${detail === undefined ? "" : `: ${JSON.stringify(detail)}`}`);
}
const note = (step) => { proof.steps.push(step); console.log(`ok  ${step}`); };

function baseEnv(platform) {
  return {
    ...process.env,
    TINE_GRAPH: GRAPH,
    XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
    XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
    XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    LIBGL_ALWAYS_SOFTWARE: "1",
    GDK_BACKEND: "x11",
    TINE_E2E_FORCE_MOBILE_DRAWERS: "1",
    TINE_E2E_TOUCH_GESTURES: platform,
  };
}

async function withApp(index, platform, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const log = fs.openSync(path.join(ARTIFACT, `driver-${index}-${platform}.log`), "w");
  const driver = spawn(DRIVER, [
    "--port", String(driverPort),
    "--native-port", String(nativePort),
    "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
  ], { env: baseEnv(platform), detached: true, stdio: ["ignore", log, log] });
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

// ---- iOS: edge swipe (drawer pull + Back) and block swipes ------------------
await withApp(0, "ios", async (browser) => {
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
  await swipe(browser, { x: 10, y: Y, path: steps(220) });
  await browser.waitUntil(async () => (await currentPageTitle(browser)) !== "Swipe target", { timeout: 8_000, timeoutMsg: "a long edge swipe did not go back" });
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
  await swipe(browser, { x: Math.max(160, mrect.x + 160), y: mrect.y + mrect.h / 2, path: steps(-110), selector: menuSel });
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
await withApp(1, "android", async (browser) => {
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
});

proof.finishedAt = new Date().toISOString();
proof.result = "pass";
const proofPath = path.join(ARTIFACT, "proof.json");
fs.writeFileSync(proofPath, `${JSON.stringify(proof, null, 2)}\n`);
console.log(`PASS: native touch gestures (${proof.touchEvents}); proof: ${proofPath}`);
