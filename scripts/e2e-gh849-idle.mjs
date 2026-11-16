// GH #849 and #848 on native WebKitGTK and hosted WebView2. A hub page is referenced from 93 source pages, more than
// the document working set holds (80). Walking Linked References from end to end loads them all.
// #849: an idle page must reach a fixed point. References used to evict and reload one another for ever, so rows
// changed form and the scrollbar crept. Sampling: every animation frame, scrollTop, scrollHeight and each reference
// group's height; plus a MutationObserver over the scroller. Idling starts once the references have been quiet for
// QUIET_MS after the jump to the middle (a few seconds of reloading near rows is expected; never settling is the bug).
// #848: from a deep position, switching to another tab and back shows the same first row, within a few px.
// Run under xvfb-run -s "-screen 0 1920x1080x24" with TINE_APP set. GH849_IDLE_SECONDS shortens the idle.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4606);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4607);
const OUT = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/gh849-idle");
const TMP = path.join(OUT, `native-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const IDLE_SECONDS = Number(process.env.GH849_IDLE_SECONDS || 40);
const SETTLE_MS = Number(process.env.GH849_SETTLE_MS || 2000);
const QUIET_MS = 3000;
const SETTLE_CAP_MS = 30_000;
const ROW_LIMIT = 8; // px the first row may move when coming back
const SOURCE_PAGES = 93;
fs.mkdirSync(OUT, { recursive: true });

for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache", "home"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{:tine/guide-announced? true}\n");
// One white pixel, drawn large: an image whose height depends on loading, as in the report.
fs.writeFileSync(`${GRAPH}/assets/shot.png`, Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==", "base64"));
const LONG = "A long sentence that wraps over several lines, so the row's height depends on real layout and on its live form. ".repeat(4);
const rowBody = (p, r) => {
  const kind = (p + r) % 5;
  if (kind === 0) return `- Row ${p}/${r} on [[Hub]] with a quote\n  > ${LONG}\n`;
  if (kind === 1) return `- Row ${p}/${r} on [[Hub]] with code\n  \`\`\`js\n${Array.from({ length: 12 }, (_, i) => `  const value${i} = ${i} * ${p};`).join("\n")}\n  \`\`\`\n`;
  if (kind === 2) return `- Row ${p}/${r} on [[Hub]] with an image\n  ![shot](../assets/shot.png){:width 400 :height 240}\n`;
  if (kind === 3) return `- Row ${p}/${r} on [[Hub]] with a citation [source](https://example.org/${p}) and $$x^${p}$$\n`;
  return `- Row ${p}/${r} on [[Hub]]. ${LONG}\n`;
};
fs.writeFileSync(`${GRAPH}/pages/Hub.md`, "- Hub intro\n");
fs.writeFileSync(`${GRAPH}/pages/Other tab.md`, "- The tab the reader switches to\n");
// 127 blocks on 93 pages: the first 34 pages have two rows, each with a child.
for (let p = 0; p < SOURCE_PAGES; p++) {
  const rows = p < 34 ? 2 : 1;
  const text = Array.from({ length: rows }, (_, r) => `${rowBody(p, r)}\t- Child of ${p}/${r}\n`).join("");
  fs.writeFileSync(`${GRAPH}/pages/Refpage ${String(p).padStart(2, "0")}.md`, text);
}

const env = { ...process.env, HOME: `${TMP}/xdg/home`, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  APPDATA: path.join(TMP, "appdata"), LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
if (process.platform === "win32" && process.env.CI === "true") spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(OUT, "gh849-tauri-driver.log"), "w");
const driver = spawn(TD, webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: webviewTarget.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
await sleep(2500);

const reading = () => {
  const s = document.querySelector(".main-content"), top = s.getBoundingClientRect().top;
  for (const row of s.querySelectorAll(".linked-references .ls-block")) {
    const r = row.getBoundingClientRect();
    if (r.top >= top && r.height > 0) return { text: row.textContent.slice(0, 40), offset: Math.round(r.top - top), top: Math.round(s.scrollTop), height: s.scrollHeight };
  }
  return { text: "", offset: 0, top: Math.round(s.scrollTop), height: s.scrollHeight };
};

let browser, failure = "";
const result = {};
try {
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 90_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress) });
  await ensureMainWindow(browser);
  try { await browser.setWindowRect(0, 0, 1280, 1000); } catch (e) { console.log(`setWindowRect: ${e.message}`); }
  await sleep(1000);
  await openPageByName(browser, "Other tab");
  await openPageByName(browser, "Hub", { timeout: 60_000 });
  // A section this long starts folded.
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".linked-references .references-header")), { timeout: 60_000, timeoutMsg: "Linked References never showed" });
  await browser.execute(() => { if (!document.querySelector(".linked-references .reference-blocks")) document.querySelector(".linked-references .references-header").click(); });
  await browser.waitUntil(() => browser.execute((n) => document.querySelectorAll(".linked-references .reference-blocks").length >= n, SOURCE_PAGES), { timeout: 60_000, timeoutMsg: `expected ${SOURCE_PAGES} reference groups` });
  // A background tab, with the link gesture a user has.
  await browser.execute(() => document.querySelector(".linked-references button.reference-page").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ctrlKey: true })));
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll('[data-tab-strip-pane-id="main"] .tab').length > 1), { timeout: 10_000, timeoutMsg: "the background tab never opened" });

  // Walk to the end as a reader does, so every source page loads, then come back to the middle.
  for (let i = 0; i < 400; i++) {
    const at = await browser.execute(() => { const s = document.querySelector(".main-content"); s.scrollTop += 1200; return (s.scrollTop + s.clientHeight) / s.scrollHeight; });
    await sleep(100);
    if (at > 0.995) break;
  }
  await browser.execute(() => { const s = document.querySelector(".main-content"); s.scrollTop = s.scrollHeight / 2; });
  // Rows near the new position that were evicted during the walk load again: a burst of a few seconds. The page
  // must then reach a fixed point: wait for QUIET_MS without a mutation (at most SETTLE_CAP_MS), then idle.
  await browser.execute(() => {
    const quiet = window.__quiet = { last: performance.now() };
    new MutationObserver(() => { quiet.last = performance.now(); })
      .observe(document.querySelector(".main-content"), { childList: true, subtree: true, characterData: true, attributes: true });
  });
  const settleStart = Date.now();
  await sleep(SETTLE_MS);
  while (Date.now() - settleStart < SETTLE_CAP_MS) {
    if (await browser.execute((ms) => performance.now() - window.__quiet.last > ms, QUIET_MS)) break;
    await sleep(500);
  }
  result.settleMs = Date.now() - settleStart;
  console.log(JSON.stringify({ settleMs: result.settleMs }));
  if (result.settleMs >= SETTLE_CAP_MS) failure += `#849: references were still changing ${SETTLE_CAP_MS / 1000} s after the reader stopped. `;

  // #849
  await browser.execute(() => {
    const s = document.querySelector(".main-content");
    const rec = window.__idle = { frames: 0, events: [], eventCount: 0, mutations: 0, sample: [], running: true };
    const heights = () => { const m = new Map(); for (const el of s.querySelectorAll(".reference-blocks")) m.set(el.dataset.inpageFindSurface, Math.round(el.getBoundingClientRect().height * 10) / 10); return m; };
    let last = { top: s.scrollTop, height: s.scrollHeight, rows: heights() };
    rec.perSecond = {};
    const t0 = performance.now();
    new MutationObserver((records) => {
      rec.mutations += records.length;
      const second = Math.floor((performance.now() - t0) / 1000);
      rec.perSecond[second] = (rec.perSecond[second] ?? 0) + records.length;
      if (rec.sample.length < 40) for (const m of records.slice(0, 3)) rec.sample.push({ t: Math.round(performance.now() - t0), type: m.type, target: String(m.target.className || m.target.nodeName).slice(0, 60), attribute: m.attributeName, old: m.oldValue, now: m.attributeName ? m.target.getAttribute(m.attributeName) : undefined, group: m.target.closest?.(".reference-group")?.querySelector(".reference-page")?.textContent ?? m.target.parentElement?.closest(".reference-group")?.querySelector(".reference-page")?.textContent });
    }).observe(s, { childList: true, subtree: true, characterData: true, attributes: true, attributeOldValue: true });
    const tick = () => {
      rec.frames++;
      const rows = heights();
      let changed = 0;
      for (const [k, h] of rows) if (last.rows.get(k) !== h) changed++;
      if (s.scrollTop !== last.top || s.scrollHeight !== last.height || changed) {
        rec.eventCount++;
        if (rec.events.length < 40) rec.events.push({ t: Math.round(performance.now() - t0), top: s.scrollTop, height: s.scrollHeight, changed });
      }
      last = { top: s.scrollTop, height: s.scrollHeight, rows };
      if (rec.running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  await sleep(IDLE_SECONDS * 1000);
  result.idle = await browser.execute(() => { const r = window.__idle; r.running = false; return r; });
  console.log(JSON.stringify({ ua: await browser.execute(() => navigator.userAgent), frames: result.idle.frames, geometry: result.idle.eventCount, mutations: result.idle.mutations, first: result.idle.sample.slice(0, 4) }));
  if (result.idle.frames < 10) failure += `the idle sampler saw only ${result.idle.frames} frames. `;
  if (result.idle.eventCount || result.idle.mutations) failure += `#849: the idle page kept changing: ${result.idle.eventCount} geometry changes, ${result.idle.mutations} mutations. `;

  // #848, at two depths: a tab switch while everything is loaded.
  for (const fraction of [0.5, 0.85]) {
    await browser.execute((f) => { const s = document.querySelector(".main-content"); s.scrollTop = s.scrollHeight * f; }, fraction);
    await sleep(3000);
    const before = await browser.execute(reading);
    await browser.execute(() => document.querySelectorAll('[data-tab-strip-pane-id="main"] .tab')[1].click());
    await sleep(1500);
    await browser.execute(() => document.querySelectorAll('[data-tab-strip-pane-id="main"] .tab')[0].click());
    await sleep(6000);
    const after = await browser.execute(reading);
    (result.tabs ??= []).push({ fraction, before, after });
    console.log(JSON.stringify({ fraction, before, after }));
    if (!before.text) failure += `#848: no reference row in view at ${fraction}. `;
    if (after.text !== before.text || Math.abs(after.offset - before.offset) > ROW_LIMIT) failure += `#848 at ${fraction}: ${JSON.stringify(before)} became ${JSON.stringify(after)}. `;
  }
} catch (error) {
  failure += String(error);
} finally {
  fs.writeFileSync(path.join(OUT, "gh849-result.json"), JSON.stringify(result, null, 1));
  await browser?.deleteSession().catch(() => {});
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
    if (process.env.CI === "true") spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
  } else try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(webviewTarget);
}
if (failure) { console.error(`FAIL: ${failure}`); process.exit(1); }
console.log("PASS: the idle page reached a fixed point and a tab switch kept the first row");
