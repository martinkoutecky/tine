// Linux real-app proof for "changed since you last looked" (vision decision 9a,
// ADR 0073): mark a page seen, quit, edit the page file outside Tine, reopen.
// Exactly the externally edited block carries the change bar and the header
// counts one change; the seen record exists in app data, and the graph folder
// gained nothing (no file added, and the page holds only the external edit).
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
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4512);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4513);
const TMP = "/tmp/tine-seen-baseline-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Seen Test.md`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const ORIGINAL = [
  "- Seen alpha stays",
  "  - Seen beta child stays",
  "- Seen gamma will change",
  "- Seen delta stays",
  "",
].join("\n");
const EDITED = ORIGINAL.replace("Seen gamma will change", "Seen gamma changed by another tool");
fs.writeFileSync(PAGE, ORIGINAL);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Seen Test]]\n");

const baseEnv = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};

function filesUnder(dir) {
  if (!fs.existsSync(dir)) return [];
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...filesUnder(full));
    else out.push(full);
  }
  return out.sort();
}

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

const seenRecords = () => filesUnder(`${TMP}/xdg`).filter((file) => /\/seen\/[^/]+\/[0-9a-f]{64}\.bin$/.test(file));

async function waitForSeenRecord() {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const records = seenRecords();
    if (records.length === 1 && fs.statSync(records[0]).size > 16) return records[0];
    await sleep(200);
  }
  throw new Error(`Mark page seen wrote no seen record under app data (found ${JSON.stringify(seenRecords())})`);
}

const seenState = (browser) => browser.execute(() => ({
  header: document.querySelector("[data-seen-header]")?.textContent?.replace(/\s+/g, " ").trim() ?? null,
  changed: [...document.querySelectorAll(".ls-block")]
    .filter((row) => row.querySelector(":scope > .block-main")?.classList.contains("seen-changed"))
    .map((row) => (row.querySelector(":scope > .block-main .block-content")?.textContent ?? "").trim()),
  rows: document.querySelectorAll(".ls-block").length,
}));

async function clickMenuItem(browser, label) {
  await browser.waitUntil(() => browser.execute((wanted) => {
    const item = [...document.querySelectorAll('[role="menuitem"]')].find((node) => (node.textContent ?? "").trim() === wanted);
    if (!(item instanceof HTMLElement)) return false;
    item.click();
    return true;
  }, label), { timeout: 10_000, interval: 150, timeoutMsg: `no ${label} menu item` });
}

const graphBefore = filesUnder(GRAPH);

// Session 1: an untracked page shows nothing; Mark page seen writes one app-data record.
await withApp(0, async (browser) => {
  await openPageByName(browser, "Seen Test");
  await browser.waitUntil(async () => (await seenState(browser)).rows >= 4, { timeout: 10_000, timeoutMsg: "the page did not render" });
  const untracked = await seenState(browser);
  if (untracked.header !== null || untracked.changed.length) throw new Error(`an untracked page shows seen chrome: ${JSON.stringify(untracked)}`);
  await browser.execute(() => {
    const trigger = document.querySelector(".main-content [data-page-actions-trigger]") ?? document.querySelector("[data-page-actions-trigger]");
    trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
  });
  await clickMenuItem(browser, "Mark page seen");
  await waitForSeenRecord();
  const marked = await seenState(browser);
  if (marked.header !== null || marked.changed.length) throw new Error(`a freshly marked page shows changes: ${JSON.stringify(marked)}`);
});

// Between sessions: another tool edits one block.
fs.writeFileSync(PAGE, EDITED);

// Session 2: exactly the edited block is highlighted, and the header counts it.
await withApp(1, async (browser) => {
  await openPageByName(browser, "Seen Test");
  await browser.waitUntil(async () => (await seenState(browser)).header !== null, {
    timeout: 15_000, timeoutMsg: "the reopened page shows no 'changes since you last looked' header",
  });
  const state = await seenState(browser);
  if (JSON.stringify(state.changed) !== JSON.stringify(["Seen gamma changed by another tool"])) {
    throw new Error(`expected exactly the externally edited block to be highlighted: ${JSON.stringify(state)}`);
  }
  if (!state.header.startsWith("1 change since you last looked")) throw new Error(`header does not count one change: ${state.header}`);
});

const record = seenRecords();
if (record.length !== 1) throw new Error(`expected one seen record in app data: ${JSON.stringify(record)}`);
const graphAfter = filesUnder(GRAPH);
if (JSON.stringify(graphAfter) !== JSON.stringify(graphBefore)) {
  throw new Error(`the graph folder changed:\nbefore ${JSON.stringify(graphBefore)}\nafter  ${JSON.stringify(graphAfter)}`);
}
if (fs.readFileSync(PAGE, "utf8") !== EDITED) throw new Error("Tine rewrote the page file");
console.log(`seen baseline: record ${path.relative(TMP, record[0])}; exactly the externally edited block highlighted after a restart; graph folder unchanged`);
