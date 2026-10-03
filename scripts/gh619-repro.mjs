// GH #619 repro driver. env: APP, KEYS (distinct property keys), WIN ("1280x900"), OUT (dir), GRAPH_SRC (copy an existing graph instead)
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
const WDIO_ROOT = "/aux/koutecky/logseq/tine-og-gh619";
const APP = process.env.APP;
const KEYS = Number(process.env.KEYS ?? 300);
const [W, H] = (process.env.WIN ?? "1280x900").split("x").map(Number);
const OUT = process.env.OUT ?? "/tmp/gh619-out"; fs.mkdirSync(OUT, { recursive: true });
const TD = "/aux/koutecky/logseq/.toolchain/cargo/bin/tauri-driver";
const PORT = Number(process.env.PORT ?? 4600);
const TMP = fs.mkdtempSync("/aux/koutecky/logseq/tmp-gh619-");
const GRAPH = `${TMP}/graph`;
for (const d of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${d}`, { recursive: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${d}`, { recursive: true });
if (process.env.GRAPH_SRC) fs.cpSync(process.env.GRAPH_SRC, GRAPH, { recursive: true });
else {
  fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
  const n = new Date(); const j = `${n.getFullYear()}_${String(n.getMonth()+1).padStart(2,"0")}_${String(n.getDate()).padStart(2,"0")}`;
  fs.writeFileSync(`${GRAPH}/journals/${j}.md`, "- first\n");
  fs.writeFileSync(`${GRAPH}/pages/Tasks.md`, "- TODO alpha\n- DONE beta\n");
  for (let i = 0; i < KEYS; i++) {
    const page = `Page ${String(i).padStart(4,"0")}`;
    fs.writeFileSync(`${GRAPH}/pages/${page}.md`, `prop-key-${String(i).padStart(4,"0")}:: v${i}\n\n- block with\n  blk-key-${i}:: x\n`);
  }
}
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(`${TMP}/td.log`, "w");
const td = spawn(TD, ["--port", String(PORT), "--native-port", String(PORT+1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
await sleep(2500);
const report = {};
let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port: PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.setWindowRect(0, 0, W, H).catch(() => {});
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30000 });
  await sleep(1500);
  report.viewport = await browser.execute(() => [innerWidth, innerHeight]);
  await browser.execute(() => { for (const c of document.querySelectorAll(".toast-sticky .toast-close")) c.click(); });
  // type /query in a fresh block on the journal page
  await browser.execute(() => { const t = document.querySelector(".page-trailing-block-target"); if (t) t.click(); else document.querySelector(".ls-block .block-content, .ls-block")?.click(); });
  const editor = await browser.$("textarea");
  await editor.waitForExist({ timeout: 10000 });
  await editor.addValue("/query");
  await sleep(800);
  await browser.saveScreenshot(`${OUT}/01-slash-menu.png`);
  if (process.env.TRACE) await browser.execute(() => {
    window.__tr = [];
    const has = (n) => n && n.nodeType === 1 && (n.matches?.(".query-block,.qs-builder") || n.querySelector?.(".qs-builder"));
    const stack = () => new Error().stack.split("\n").slice(2, 9).map(l => l.trim().replace(/https?:\/\/[^ )]*\//, "")).join(" | ");
    for (const m of ["removeChild", "replaceChild"]) { const o = Node.prototype[m]; Node.prototype[m] = function (...a) { if (has(a[0]) || has(a[1])) window.__tr.push(m + " " + (a[0]?.className||"") + " connected=" + this.isConnected + " :: " + stack()); return o.apply(this, a); }; }
    const tc = Object.getOwnPropertyDescriptor(Node.prototype, "textContent"); Object.defineProperty(Node.prototype, "textContent", { ...tc, set(v) { if (v === "" && this.querySelector?.(".qs-builder")) window.__tr.push("textContent='' on " + this.className + " :: " + stack()); return tc.set.call(this, v); } });
    const ins = Node.prototype.insertBefore; Node.prototype.insertBefore = function (n, r) { if (n?.nodeType === 1 && (n.matches?.(".qs-builder") || n.querySelector?.(".qs-builder"))) window.__tr.push("insert qs-builder into " + this.className + " connected=" + this.isConnected + " :: " + stack()); return ins.call(this, n, r); };
  });
  await browser.keys(["Enter"]);
  await sleep(2500);
  if (process.env.TRACE) report.trace = await browser.execute(() => window.__tr);
  await browser.saveScreenshot(`${OUT}/02-after-query-cmd.png`);
  report.sheet = await browser.execute(() => {
    const s = document.querySelector(".qs-sheet");
    const r = s?.getBoundingClientRect();
    const rows = [...document.querySelectorAll(".qs-vocab-row")].map(e => Math.round(e.getBoundingClientRect().height));
    const opts = document.querySelector(".qs-vocab-options");
    const add = document.querySelector(".qs-sheet .qs-add");
    return { sheetRect: r && [r.x, r.y, r.width, r.height].map(Math.round), addExpanded: add?.getAttribute("aria-expanded"),
      rowCount: rows.length, rowHeights: rows.slice(0, 20), optsRect: opts && (()=>{const q=opts.getBoundingClientRect();return [q.x,q.y,q.width,q.height].map(Math.round)})(),
      optsScroll: opts && [opts.scrollHeight, opts.clientHeight], active: document.activeElement?.tagName + "." + document.activeElement?.className,
      resultText: (document.querySelector(".qs-sheet")?.textContent ?? "").slice(0, 300) };
  });
  // Hit-test: element at the centre of several sheet controls
  report.hit = await browser.execute(() => {
    const out = {};
    for (const sel of [".qs-sheet .qs-add", ".qs-sheet .qs-menu-filter", ".qs-vocab-option", ".qs-sheet .query-text-pane-input", ".qs-sheet"]) {
      const e = document.querySelector(sel); if (!e) { out[sel] = null; continue; }
      const r = e.getBoundingClientRect(); const x = r.x + r.width/2, y = r.y + Math.min(r.height/2, 10);
      const top = document.elementFromPoint(x, y);
      out[sel] = { at: [Math.round(x), Math.round(y)], top: top && (top.tagName + "." + top.className).slice(0, 80), inside: !!top && (e === top || e.contains(top)) };
    }
    return out;
  });
  // Escape closes only chooser
  await browser.keys(["Escape"]); await sleep(600);
  await browser.saveScreenshot(`${OUT}/03-after-escape.png`);
  report.afterEscape = await browser.execute(() => ({ sheet: !!document.querySelector(".qs-sheet"), add: document.querySelector(".qs-sheet .qs-add")?.getAttribute("aria-expanded") }));
  await browser.execute(() => { window.__ev = []; for (const t of ["pointerdown","mousedown","mouseup","click","focusin"]) document.addEventListener(t, (e) => { const tg = e.target; window.__ev.push(t + " " + (tg?.tagName||"") + "." + String(tg?.className||"").slice(0,40) + " sheetHas=" + !!document.querySelector(".qs-sheet") + " y=" + (e.clientY??"")); }, true); });
  report.hostChain = await browser.execute(() => { const c = document.querySelector(".qs-sheet-anchor")?.parentElement; let h = c?._$host; const a = []; let n = h; while (n && a.length < 12) { a.push((n.tagName||"#")+"."+String(n.className||"").slice(0,30)); n = n._$host || n.parentNode; } return { hostConnected: h?.isConnected, chain: a, editing: !!document.querySelector("textarea.block-editor") }; });
  report.preClick = await browser.execute(() => { const e = document.querySelector(".qs-sheet-anchor"); const cs = getComputedStyle(e); const r = e.getBoundingClientRect(); return { pos: cs.position, rect: [r.x,r.y,r.width,r.height].map(Math.round), scrollY: scrollY, parents: (()=>{let a=[],n=e;while(n&&a.length<5){a.push(n.tagName+"."+n.className);n=n.parentElement}return a})() }; });
  // MOUSE: click + Add condition, then type to filter, click a row
  const add = await browser.$(".qs-sheet .qs-add");
  if (await add.isExisting()) {
    try { await add.click(); report.addClick = "ok"; } catch (e) { report.addClick = "ERR " + String(e.message).slice(0, 200); }
    await sleep(600);
    report.events = await browser.execute(() => window.__ev);
    await browser.saveScreenshot(`${OUT}/04-after-add-click.png`);
    report.afterAddClick = await browser.execute(() => ({ sheet: !!document.querySelector(".qs-sheet"), expanded: document.querySelector(".qs-sheet .qs-add")?.getAttribute("aria-expanded"),
      editing: document.activeElement?.tagName, rows: [...document.querySelectorAll(".qs-vocab-row")].slice(0, 8).map(e => Math.round(e.getBoundingClientRect().height)) }));
    const opt = await browser.$(".qs-vocab-option");
    if (await opt.isExisting()) {
      const before = await browser.execute(() => document.querySelector(".qs-sheet")?.textContent?.slice(0, 200));
      try { await opt.click(); report.optClick = "ok"; } catch (e) { report.optClick = "ERR " + String(e.message).slice(0, 200); }
      await sleep(800);
      await browser.saveScreenshot(`${OUT}/05-after-option-click.png`);
      report.afterOpt = await browser.execute(() => ({ sheet: !!document.querySelector(".qs-sheet"), text: document.querySelector(".qs-sheet")?.textContent?.slice(0, 300), activeEditor: !!document.querySelector(".page-blocks textarea") }));
    }
  }
  // KEYBOARD: type in filter
  report.final = await browser.execute(() => ({ sheet: !!document.querySelector(".qs-sheet"), body: document.body.innerText.slice(0, 200) }));
} catch (e) { report.error = String(e.stack ?? e).slice(0, 800); try { await browser.saveScreenshot(`${OUT}/error.png`); } catch {} }
finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.writeFileSync(`${OUT}/report.json`, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  fs.rmSync(TMP, { recursive: true, force: true });
}
