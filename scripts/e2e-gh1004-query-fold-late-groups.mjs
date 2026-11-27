// Native journey for GH #1004: "Collapse (one level)" on a selected query block
// folds every result, including the page groups not drawn yet at the press.
// 30 source pages, one ticket with one child each; the query is on its own
// routed page. The press runs once from the command palette and once from a
// chord, each on a fresh page (a fresh result list). Result folds are local
// presentation: no file changes.
import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { remote } from "webdriverio";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/gh1004");
const app = process.env.TINE_APP || path.join(root, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const driverPort = +(process.env.E2E_DRIVER_PORT || 4714);
const nativePort = +(process.env.E2E_NATIVE_PORT || 4715);
const run = fs.mkdtempSync(path.join(os.tmpdir(), "tine-gh1004-"));
fs.mkdirSync(out, { recursive: true });
const graph = path.join(run, "graph");
for (const dir of ["logseq", "pages", "journals"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"),
  '{:tine/guide-announced? true :shortcuts {:editor/collapse-children "mod+alt+k"}}\n');
const N = 30;
for (let i = 1; i <= N; i++) {
  fs.writeFileSync(path.join(graph, `pages/Source ${String(i).padStart(2, "0")}.md`), `- Ticket ${i} #Ticket\n  - Child ${i}\n`);
}
const MODES = ["palette", "chord"];
for (const mode of MODES) fs.writeFileSync(path.join(graph, `pages/Q ${mode}.md`), "- Intro\n- {{query (and [[Ticket]])}}\n");
const digest = () => {
  const hash = crypto.createHash("sha256");
  for (const dir of ["pages", "journals", "logseq"]) for (const name of fs.readdirSync(path.join(graph, dir)).sort()) {
    const file = path.join(graph, dir, name);
    if (fs.statSync(file).isFile()) hash.update(`${dir}/${name}\0`).update(fs.readFileSync(file));
  }
  return hash.digest("hex");
};
const before = digest();

const fixture = accountHomeFixture(run, out, {
  ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"),
  APPDATA: path.join(run, "appdata"), LOCALAPPDATA: path.join(run, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", GDK_BACKEND: "x11",
});
const launcher = await startWebdriverApplication(app, fixture.env, nativePort, "gh1004");
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(driverPort, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: launcher.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });

/** Ticket rows drawn now: number → whether its child is visible. Plus how many page groups are drawn. */
function rows() {
  const own = el => (el.querySelector(":scope > .block-main .block-content-wrapper")?.textContent ?? "").trim();
  const seen = {};
  for (const el of document.querySelectorAll(".main-content .query-group .ls-block")) {
    const m = /^Ticket (\d+)\b/.exec(own(el));
    if (!m) continue;
    const child = [...el.querySelectorAll(".ls-block")].find(c => own(c) === `Child ${m[1]}`);
    seen[m[1]] = !!child && child.getBoundingClientRect().height > 0;
  }
  const groups = [...document.querySelectorAll(".main-content .query-group")];
  return { seen, groups: groups.length, drawn: groups.filter(g => g.getAttribute("data-find-source-mounted") === "true").length };
}
const scrollBy = (to) => {
  const s = document.querySelector(".main-content");
  s.scrollTop = to === "top" ? 0 : s.scrollTop + to;
  return { top: s.scrollTop, max: s.scrollHeight - s.clientHeight, h: s.clientHeight };
};
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

let browser;
let phase = "driver";
const proof = { modes: {} };
async function stable() {
  let last = "";
  await browser.waitUntil(async () => {
    const now = JSON.stringify(await browser.execute(rows));
    const same = now === last; last = now; return same;
  }, { timeout: 20_000, interval: 300, timeoutMsg: "result rows never settled" });
}
/** Scroll through the whole list; the last state seen of each ticket wins. */
async function sweep() {
  const seen = {};
  await browser.execute(scrollBy, "top"); await stable();
  for (let guard = 0; guard < 200; guard++) {
    Object.assign(seen, (await browser.execute(rows)).seen);
    const pos = await browser.execute(scrollBy, 0);
    if (pos.top >= pos.max - 1) break;
    await browser.execute(scrollBy, Math.floor(pos.h * 0.6)); await stable();
  }
  Object.assign(seen, (await browser.execute(rows)).seen);
  return seen;
}
async function selectQuery() {
  const intro = await browser.execute(() => [...document.querySelectorAll(".main-content .ls-block")]
    .find(el => (el.querySelector(":scope > .block-main .block-content-wrapper")?.textContent ?? "").trim() === "Intro")?.getAttribute("data-block-id"));
  if (!intro) throw new Error("Intro block not found");
  await browser.execute(id => {
    const el = document.querySelector(`[data-block-id="${id}"] > .block-main .block-content-wrapper`);
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  }, intro);
  await browser.waitUntil(() => browser.execute(id => !!document.querySelector(`[data-block-id="${id}"] > .block-main.editing, [data-block-id="${id}"] > .block-main textarea, [data-block-id="${id}"] > .block-main .cm-editor`), intro),
    { timeout: 10_000, timeoutMsg: "Intro did not open for editing" });
  await browser.keys(["Escape"]);
  await browser.keys(["ArrowDown"]);
  await browser.waitUntil(() => browser.execute(() => {
    const host = document.querySelector(".main-content .query-group")?.closest(".ls-block");
    return !!host?.querySelector(":scope > .block-main.selected");
  }), { timeout: 5_000, timeoutMsg: "the query block was not selected" });
}
async function collapse(mode) {
  if (mode === "chord") { await browser.keys(["Control", "Alt", "k", "NULL"]); return; }
  const label = "Collapse (one level)";
  await browser.keys(["Control", "Shift", "p"]);
  const input = await browser.$(".switcher-input"); await input.waitForDisplayed({ timeout: 10_000 });
  await input.setValue(label);
  await browser.waitUntil(() => browser.execute(l => [...document.querySelectorAll(".switcher-row")]
    .some(r => (r.querySelector(".switcher-name")?.textContent ?? r.textContent).trim() === l), label), { timeout: 10_000, timeoutMsg: "palette row missing" });
  const index = await browser.execute(l => [...document.querySelectorAll(".switcher-row")]
    .findIndex(r => (r.querySelector(".switcher-name")?.textContent ?? r.textContent).trim() === l), label);
  await (await browser.$$(".switcher-row"))[index].click();
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".switcher-input")), { timeout: 10_000, timeoutMsg: "palette stayed open" });
}

try {
  const deadline = Date.now() + 20_000;
  while ((await fetch(`http://127.0.0.1:${driverPort}/status`).catch(() => null))?.ok !== true) {
    if (driver.exitCode !== null || Date.now() >= deadline) throw new Error("WebDriver did not start");
    await pause(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/",
    logLevel: "error", connectionRetryCount: 0, capabilities: tauriCapabilities(app, "gh1004", process.platform, launcher.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setTimeout({ script: 20_000 });
  try { await browser.setWindowRect(0, 0, 1280, 800); } catch (error) { console.log(error.message); }
  await assertAppLease(fixture);
  for (const mode of MODES) {
    phase = `${mode}: open the query page`;
    await openPageByName(browser, `Q ${mode}`);
    await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".main-content .query-group .ls-block")),
      { timeout: 30_000, timeoutMsg: "no result rows rendered" });
    await stable();
    phase = `${mode}: select the query`;
    await selectQuery();
    const atPress = await browser.execute(rows);
    if (atPress.drawn >= atPress.groups) throw new Error(`every group was drawn at the press (${atPress.drawn}/${atPress.groups}); the fixture proves nothing`);
    phase = `${mode}: collapse`;
    await collapse(mode);
    await browser.waitUntil(async () => Object.values((await browser.execute(rows)).seen).some(open => !open),
      { timeout: 10_000, timeoutMsg: "no result folded" });
    phase = `${mode}: scroll through the results`;
    const seen = await sweep();
    const open = Object.entries(seen).filter(([, visible]) => visible).map(([n]) => +n);
    const missing = Array.from({ length: N }, (_, i) => String(i + 1)).filter(n => !(n in seen)).map(Number);
    const host = await browser.execute(() => !!document.querySelector(".main-content .query-group"));
    proof.modes[mode] = { atPress: { groups: atPress.groups, drawn: atPress.drawn }, open, missing, queryOpen: host };
    if (!host) throw new Error(`${mode}: the query block folded itself`);
    if (missing.length) throw new Error(`${mode}: tickets never drawn while scrolling: ${missing.join(", ")}`);
    if (open.length) throw new Error(`${mode}: tickets still showing their child after Collapse: ${open.join(", ")}`);
  }
  phase = "files";
  if (digest() !== before) throw new Error("a graph file changed on disk");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  console.log(`PASS: Collapse (one level) folded all ${N} results, including groups not drawn at the press; files unchanged`);
} catch (error) {
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({
    commit: process.env.TINE_SOURCE_REVISION || "working tree", journey: "GH #1004 query fold reaches late groups",
    phase, proof, expected: "After Collapse (one level) on the selected query and a scroll through it, no ticket's child is visible, the query stays open, files are unchanged",
    observed: String(error), classification: phase === "driver" ? "infrastructure" : "ambiguous", log: "driver.log",
  }, null, 2) + "\n");
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  driver.kill();
  stopWebdriverApplication(launcher);
  fixture.close?.();
  fixture.verify();
  fs.closeSync(log);
}
