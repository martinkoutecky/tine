// Linux real-app proof for GH #623 (browsing never mutates the graph). Zooming into
// an ID-less block by its bullet and jumping to a block from Ctrl-K must leave the
// page file byte-for-byte unchanged (no `id::` stamped just to remember a place),
// and a fresh native process must restore the zoomed block from the session.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-browse-never-writes-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Browse Test.md`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const ORIGINAL = [
  "- Zoom parent alpha",
  "  - Zoom child beta",
  "    - Zoom grandchild gamma",
  "- Quick target delta",
  "",
].join("\n");
fs.writeFileSync(PAGE, ORIGINAL);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Browse Test]]\n");

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
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

function assertPageUntouched(when) {
  const bytes = fs.readFileSync(PAGE, "utf8");
  if (bytes !== ORIGINAL) throw new Error(`${when}: browsing changed the page file:\n${bytes}`);
}

function filesUnder(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...filesUnder(full));
    else out.push(full);
  }
  return out;
}

// The session file is written asynchronously; wait for the zoomed place to be
// recorded as a position (never an id) instead of sleeping a guessed debounce.
async function waitForSessionPosition() {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    for (const file of filesUnder(TMP + "/xdg")) {
      if (!file.endsWith(".json")) continue;
      const text = fs.readFileSync(file, "utf8");
      if (text.includes("Browse Test") && text.includes("blockPos")) return;
    }
    await sleep(200);
  }
  throw new Error("the session never recorded the zoomed block by position");
}

const zoomedText = (browser) => browser.execute(() => document.querySelector(".zoomed-page .zoomed-block")?.textContent ?? "");

async function zoomByBullet(browser, text) {
  const clicked = await browser.execute((wanted) => {
    const row = [...document.querySelectorAll(".ls-block")].find((node) =>
      (node.querySelector(":scope > .block-main .block-content")?.textContent ?? "").trim() === wanted);
    const bullet = row?.querySelector(":scope > .block-main .bullet-container");
    if (!(bullet instanceof HTMLElement)) return false;
    bullet.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    bullet.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
    bullet.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  }, text);
  if (!clicked) throw new Error(`no bullet for block ${JSON.stringify(text)}`);
}

async function jumpToBlockFromSwitcher(browser, query, blockText) {
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5000 });
  await input.setValue(query);
  // Find and activate in one round trip (never hold a handle across the list).
  await browser.waitUntil(() => browser.execute((wanted) => {
    const row = [...document.querySelectorAll(".switcher-row.block-result")]
      .find((node) => (node.textContent ?? "").includes(wanted));
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  }, blockText), { timeout: 15_000, interval: 150, timeoutMsg: `Ctrl-K never offered the block ${JSON.stringify(blockText)}` });
}

await withApp(0, async (browser) => {
  await openPageByName(browser, "Browse Test");
  assertPageUntouched("opening the page");

  await zoomByBullet(browser, "Zoom child beta");
  await browser.waitUntil(async () => (await zoomedText(browser)).includes("Zoom grandchild gamma"), {
    timeout: 10_000, timeoutMsg: "clicking the bullet did not zoom into the block",
  });
  assertPageUntouched("zooming by bullet");

  // OG parity: a Ctrl-K block result opens the page and brings the block into view
  // (it does not zoom). It must still leave no trace in the file.
  await jumpToBlockFromSwitcher(browser, "Quick target delta", "Quick target delta");
  await browser.waitUntil(() => browser.execute(() =>
    !document.querySelector(".zoomed-page")
    && [...document.querySelectorAll(".ls-block")].some((node) =>
      (node.querySelector(":scope > .block-main .block-content")?.textContent ?? "").trim() === "Quick target delta")),
  { timeout: 10_000, timeoutMsg: "Ctrl-K block result did not show the block on its page" });
  assertPageUntouched("Ctrl-K block jump");

  // End on a bullet-zoomed block with a subtree so the restart assertion is not vacuous.
  await zoomByBullet(browser, "Zoom child beta");
  await browser.waitUntil(async () => (await zoomedText(browser)).includes("Zoom grandchild gamma"), {
    timeout: 10_000, timeoutMsg: "clicking the bullet again did not zoom into the child block",
  });
  assertPageUntouched("zooming after the Ctrl-K jump");
  await waitForSessionPosition();
});

// A fresh native process restores the zoomed block from the session's position,
// and still has not touched the file.
await withApp(1, async (browser) => {
  await browser.waitUntil(async () => (await zoomedText(browser)).includes("Zoom grandchild gamma"), {
    timeout: 15_000, timeoutMsg: `restart did not restore the zoomed block (zoomed text: ${JSON.stringify(await zoomedText(browser))})`,
  });
  assertPageUntouched("restoring the session");
});

console.log("PASS: zoom and Ctrl-K block navigation wrote nothing to the graph, and a restart restored the zoomed block");
