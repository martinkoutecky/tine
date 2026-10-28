// "Changed since you last looked" (ADR 0079) on a disposable git graph with two authors:
//  1. Opening the page right after the graph's first open shows nothing.
//  2. Another author edits one block and removes another (committed); the repository owner
//     ("me") also commits an edit. The page shows the margin bar on exactly the other
//     author's edited block, a removed-block marker that unfolds the removed text, and the
//     header "2 changes since you last looked": the owner's own commit never counts.
//  3. Editing a block in Tine does not read the changes again (the page's own saves are no trigger).
//  4. Leaving the page marks it seen: coming back shows nothing.
//  5. After a restart the page is still seen (the record is on this device, not in memory).
//  6. A later commit by the other author shows again; Page actions → Mark page seen clears it,
//     and Settings → Graph offers Mark whole graph seen and Forget seen state.
// Screenshots land in artifacts/seen-ui/. Usage: node scripts/e2e-seen-changes.mjs (needs git on PATH).
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";

const run = path.resolve(`artifacts/seen-changes/native/run-${Date.now()}`);
const shots = path.resolve("artifacts/seen-ui");
fs.mkdirSync(shots, { recursive: true });
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
const seenPage = path.join(graph, "pages/Seen.md");
const body = (lines) => `${lines.map((line) => `- ${line}`).join("\n")}\n`;
fs.writeFileSync(seenPage, body(["Alpha stays the same for everyone", "Beta will change in the next commit", "Gamma will be removed in the next commit", "Delta stays the same for everyone"]));
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- another page\n");
const git = (...args) => execFileSync("git", ["-C", graph, ...args], { encoding: "utf8" }).trim();
git("init", "-q", "-b", "main");
git("config", "user.name", "Me");
git("config", "user.email", "me@example.invalid");
git("add", "-A");
git("commit", "-q", "-m", "initial");
const commitAs = (name, email, message) => {
  git("add", "-A");
  git("-c", `user.name=${name}`, "-c", `user.email=${email}`, "commit", "-q", "-m", message);
};

const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const basePort = Number(process.env.E2E_DRIVER_PORT || 4602);
const baseNative = Number(process.env.E2E_NATIVE_PORT || 4603);

let session = null;
async function launch(index) {
  const port = basePort + index * 4;
  const nativePort = baseNative + index * 4;
  const target = await startWebdriverApplication(app, env, nativePort);
  const log = fs.openSync(path.join(run, `driver-${index}.log`), "w");
  const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env: target.env, stdio: ["ignore", log, log], detached: true });
  session = { target, log, driver, browser: null };
  await sleep(2500);
  const browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  session.browser = browser;
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 30_000 });
  return browser;
}
async function quit() {
  if (!session) return;
  const { browser, driver, target, log } = session;
  session = null;
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  stopWebdriverApplication(target);
  fs.closeSync(log);
  await sleep(1500);
}

// WebKitGTK's WebDriver screenshot stalls without a window manager, so grab the X display
// (the app window sits at the top left of the virtual screen).
const shot = async (name) => {
  await sleep(300);
  try { execFileSync("import", ["-window", "root", "-crop", "1280x900+0+0", "+repage", path.join(shots, name)], { env: process.env, stdio: "ignore" }); }
  catch (error) { console.log(`note: no screenshot ${name}: ${error.message}`); }
};
const state = (browser) => browser.execute(() => ({
  header: document.querySelector("[data-seen-header]")?.textContent?.replace(/\s+/g, " ").trim() ?? null,
  title: document.querySelector(".seen-header-text")?.getAttribute("title") ?? null,
  changed: [...document.querySelectorAll(".ls-block")]
    .filter((row) => row.querySelector(":scope > .block-main")?.classList.contains("seen-changed"))
    .map((row) => (row.querySelector(":scope > .block-main .block-content")?.textContent ?? "").trim()),
  // The block each removed marker follows.
  removed: [...document.querySelectorAll("[data-seen-removed]")].map((marker) => ({
    after: (marker.closest(".ls-block")?.querySelector(":scope > .block-main .block-content")?.textContent ?? "").trim(),
    open: !!marker.querySelector(".seen-removed-text"),
  })),
  rows: document.querySelectorAll(".ls-block").length,
  calls: window.__seenCalls ?? null,
  last: window.__seenLast ?? null,
}));
const calls = (browser) => browser.execute(() => ({ ...(window.__seenCalls ?? {}) }));
// Count the IPC commands the page issues. `__TAURI_INTERNALS__.invoke` is frozen, so count at the
// transport: the ipc custom-protocol fetch, and the message-handler fallback.
const countCalls = (browser) => browser.execute(() => {
  if (window.__seenCalls) return true;
  window.__seenCalls = {};
  const bump = (cmd) => { window.__seenCalls[cmd] = (window.__seenCalls[cmd] ?? 0) + 1; };
  const originalFetch = window.fetch.bind(window);
  window.fetch = (input, init) => {
    const url = typeof input === "string" ? input : input?.url ?? String(input);
    const cmd = /ipc/.test(url) ? decodeURIComponent(new URL(url).pathname.split("/").pop() ?? "") : null;
    if (cmd) bump(cmd);
    const result = originalFetch(input, init);
    // The last answer, for diagnosing a failing journey.
    if (cmd === "get_page_changes") result.then((response) => response.clone().json()).then((reply) => { window.__seenLast = { reply, sent: String(init?.body ?? "").slice(0, 600) }; }, () => {});
    return result;
  };
  const handler = window.webkit?.messageHandlers?.ipc;
  if (handler?.postMessage) {
    const original = handler.postMessage.bind(handler);
    handler.postMessage = (message) => {
      try { bump(JSON.parse(typeof message === "string" ? message : "{}").cmd); } catch {}
      return original(message);
    };
  }
  return true;
});
const nudge = (browser) => browser.execute(() => window.dispatchEvent(new Event("focus")));
async function waitFor(browser, what, predicate, { timeout = 20_000, poke = true } = {}) {
  let last = null;
  try {
    await browser.waitUntil(async () => {
      if (poke) await nudge(browser);
      last = await state(browser);
      return predicate(last);
    }, { timeout, interval: 400 });
  } catch (error) {
    throw new Error(`${what}: ${error.message}; last state ${JSON.stringify(last)}`);
  }
  return last;
}
const canon = (value) => JSON.stringify(value, (_key, v) => (v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => (a < b ? -1 : 1))) : v));
const expectEq = (what, actual, expected) => {
  if (canon(actual) !== canon(expected)) throw new Error(`${what}: expected ${canon(expected)}, got ${canon(actual)}`);
};
async function clickMenuItem(browser, label) {
  await browser.waitUntil(() => browser.execute((wanted) => {
    const item = [...document.querySelectorAll('[role="menuitem"]')].find((node) => (node.textContent ?? "").trim() === wanted);
    if (!(item instanceof HTMLElement)) return false;
    item.click();
    return true;
  }, label), { timeout: 10_000, interval: 150, timeoutMsg: `no ${label} menu item` });
}
const openPageActions = (browser) => browser.execute(() => {
  const trigger = document.querySelector(".main-content [data-page-actions-trigger]") ?? document.querySelector("[data-page-actions-trigger]");
  trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
});

let browser;
try {
  // 1. First open: nothing to show.
  browser = await launch(0);
  await openPageByName(browser, "Seen");
  await waitFor(browser, "the page never rendered", (s) => s.rows >= 4, { poke: false });
  await countCalls(browser);
  await sleep(1200);
  let s = await state(browser);
  expectEq("a freshly opened graph shows seen chrome", [s.header, s.changed, s.removed], [null, [], []]);
  console.log("PASS: nothing shows right after the graph's first open");

  // 2. Another author's commit shows; the owner's own commit does not.
  fs.writeFileSync(seenPage, body(["Alpha stays the same for everyone", "Beta will change in the next commit now", "Delta stays the same for everyone"]));
  commitAs("Jo", "jo@example.invalid", "Jo edits Beta and removes Gamma");
  fs.writeFileSync(seenPage, body(["Alpha stays the same for everyone too", "Beta will change in the next commit now", "Delta stays the same for everyone"]));
  commitAs("Me", "me@example.invalid", "I edit Alpha");
  s = await waitFor(browser, "the other author's changes never showed", (x) => x.changed.length === 1 && x.removed.length === 1);
  expectEq("blocks with the margin bar", s.changed, ["Beta will change in the next commit now"]);
  expectEq("header", s.header, "2 changes since you last lookedMark page seen");
  expectEq("the removed block's marker follows its previous sibling, folded", s.removed, [{ after: "Beta will change in the next commit now", open: false }]);
  await shot("seen-bar-header-marker.png");
  await browser.execute(() => document.querySelector("[data-seen-removed] button")?.click());
  s = await state(browser);
  expectEq("the removed marker unfolds", s.removed[0]?.open, true);
  const unfolded = await browser.execute(() => document.querySelector(".seen-removed-text")?.textContent ?? "");
  expectEq("the removed text", unfolded, "Gamma will be removed in the next commit");
  await shot("seen-removed-unfolded.png");
  console.log("PASS: the other author's edit and removal show; the owner's own commit does not count");

  // 3. Tine's own save is no trigger for a read.
  const readsBefore = (await calls(browser)).get_page_changes ?? 0;
  const delta = await browser.$("//div[contains(@class,'block-content')][contains(.,'Delta stays the same')]");
  await delta.click();
  await browser.keys(["End", " and more"]);
  await browser.keys("Escape");
  await browser.waitUntil(() => fs.readFileSync(seenPage, "utf8").includes("Delta stays the same for everyone and more"), { timeout: 15_000, timeoutMsg: "Tine never saved the edit" });
  await sleep(1500);
  const readsAfter = (await calls(browser)).get_page_changes ?? 0;
  expectEq("get_page_changes calls across an own edit and its save", readsAfter, readsBefore);
  console.log("PASS: an edit and its save in Tine cause no re-read");

  // 4. Leaving marks the page seen.
  const marksBefore = (await calls(browser)).mark_page_seen ?? 0;
  await openPageByName(browser, "Other");
  await browser.waitUntil(async () => ((await calls(browser)).mark_page_seen ?? 0) > marksBefore, { timeout: 10_000, timeoutMsg: "leaving the page never marked it seen" });
  await openPageByName(browser, "Seen");
  await waitFor(browser, "the page never rendered again", (x) => x.rows >= 3, { poke: false });
  await sleep(1500);
  s = await state(browser);
  expectEq("seen chrome after leaving and coming back", [s.header, s.changed, s.removed], [null, [], []]);
  console.log("PASS: leaving the page clears it");

  // 5. A restart keeps it cleared.
  await quit();
  browser = await launch(1);
  await openPageByName(browser, "Seen");
  await waitFor(browser, "the page never rendered after the restart", (x) => x.rows >= 3, { poke: false });
  await sleep(1500);
  s = await state(browser);
  expectEq("seen chrome after a restart", [s.header, s.changed, s.removed], [null, [], []]);
  console.log("PASS: a restart keeps the page seen");

  // 6. A later commit shows again; Mark page seen (Page actions) and the Settings controls.
  await countCalls(browser);
  fs.writeFileSync(seenPage, body(["Alpha stays the same for everyone too", "Beta will change in the next commit now", "Delta stays the same for everyone now"]));
  commitAs("Jo", "jo@example.invalid", "Jo edits Delta");
  s = await waitFor(browser, "the later commit never showed", (x) => x.changed.length === 1 && !x.removed.length);
  expectEq("blocks with the margin bar after a later commit", s.changed, ["Delta stays the same for everyone now"]);
  expectEq("header", s.header, "1 change since you last lookedMark page seen");
  await openPageActions(browser);
  await clickMenuItem(browser, "Mark page seen");
  await waitFor(browser, "Mark page seen never cleared the page", (x) => x.header === null && !x.changed.length, { poke: false });
  console.log("PASS: Page actions → Mark page seen clears the page");

  await browser.$('button.icon-btn[title^="Settings"]').click();
  await browser.waitUntil(() => browser.execute(() => {
    const item = [...document.querySelectorAll(".settings-nav-item")].find((node) => node.textContent?.trim() === "Graph");
    item?.click();
    return !!document.querySelector("[data-seen-mark-graph]") && !!document.querySelector("[data-seen-forget]");
  }), { timeout: 15_000, timeoutMsg: "Settings → Graph never offered Mark whole graph seen and Forget seen state" });
  await browser.execute(() => document.querySelector("[data-seen-mark-graph]")?.click());
  await browser.waitUntil(async () => ((await calls(browser)).mark_graph_seen ?? 0) === 1, { timeout: 10_000, timeoutMsg: "Mark whole graph seen never reached the backend" });
  await browser.execute(() => document.querySelector("[data-seen-forget]")?.click());
  await browser.waitUntil(async () => ((await calls(browser)).forget_seen_state ?? 0) === 1, { timeout: 10_000, timeoutMsg: "Forget seen state never reached the backend" });
  console.log("PASS: Settings → Graph marks the whole graph seen and forgets the seen state");
  await browser.keys("Escape");
} catch (error) {
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ journey: "seen changes", expected: "another author's commit shows bar, marker and header; leaving and restarting keep it cleared", observed: String(error), classification: "ambiguous", commit: process.env.TESTED_COMMIT || "working tree" }, null, 2));
  throw error;
} finally {
  await quit();
}
