// e2e-clickcaret-repro.mjs — REAL-APP click→caret verification (WebKitGTK).
// Drives real pointer clicks at computed text coordinates and reads the caret,
// with document.caretRangeFromPoint instrumented. Covers what the mock harness
// can't: real caretRangeFromPoint behavior, typographic (`->`/`--`) plains, and
// the blur-reflow race (click a block below a focused taller-in-edit block).
// Born from the Jul 2 2026 bug pair; run manually: node scripts/e2e-clickcaret-repro.mjs

import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/txdg-clickrepro-g";
const LOCAL_APP = path.join(ROOT, "target/release/tine");
const APP = process.env.TINE_APP || LOCAL_APP;
const CARGO_TAURI_DRIVER = process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : null;
const TD = process.env.TAURI_DRIVER ||
  (CARGO_TAURI_DRIVER && fs.existsSync(CARGO_TAURI_DRIVER) ? CARGO_TAURI_DRIVER : "tauri-driver");

let xvfb;
async function ensureDisplay() {
  if (process.env.DISPLAY) return;
  for (const display of [":97", ":98", ":99", ":100"]) {
    const logfd = fs.openSync(`/tmp/xvfb-clickrepro${display.slice(1)}.log`, "w");
    let err = "";
    const child = spawn("Xvfb", [display, "-screen", "0", "1400x1000x24"], { stdio: ["ignore", logfd, logfd] });
    child.on("error", (e) => { err = e.message; });
    await sleep(900);
    if (!err && child.exitCode == null) { xvfb = child; process.env.DISPLAY = display; return; }
  }
  throw new Error("Xvfb failed to start");
}

const PAGE = `- **bold** rest of line
  second line here
- **bold** start then a -> b and x -- y here
  more -- dashed text line
- TODO focus me
  DEADLINE: <2026-07-10 Fri>
- plain target below
- another plain block
`;

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/pages`, { recursive: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.mkdirSync(`${G}/logseq`, { recursive: true });
fs.writeFileSync(`${G}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${G}/pages/ClickRepro.md`, PAGE);
const now = new Date();
const stem = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${G}/journals/${stem}.md`, "- open [[ClickRepro]]\n");

await ensureDisplay();
fs.rmSync("/tmp/txdg-cr", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-cr/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-cr/data",
  XDG_CONFIG_HOME: "/tmp/txdg-cr/config",
  XDG_CACHE_HOME: "/tmp/txdg-cr/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};
const tdLog = fs.openSync("/tmp/td-clickrepro.log", "w");
const td = spawn(TD, ["--port", "4446", "--native-port", "4447", "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", tdLog, tdLog] });
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: 4446, path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1500);
  for (const sel of ["a.page-ref=ClickRepro", "span.page-ref=ClickRepro", "*=ClickRepro"]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) { await el.click(); console.log(`opened via: ${sel}`); break; }
  }
  await sleep(2000);

  // Instrument caretRangeFromPoint to record what WebKit actually returns.
  await browser.execute(() => {
    const orig = document.caretRangeFromPoint ? document.caretRangeFromPoint.bind(document) : null;
    window.__crfpLog = [];
    if (orig) {
      document.caretRangeFromPoint = (x, y) => {
        const r = orig(x, y);
        let desc = null;
        if (r) {
          const c = r.startContainer;
          const el = c.nodeType === 3 ? c.parentElement : c;
          desc = {
            nodeType: c.nodeType,
            text: c.nodeType === 3 ? (c.textContent || "").slice(0, 30) : null,
            tag: el ? el.tagName : null,
            cls: el ? String(el.className).slice(0, 40) : null,
            so: el ? el.getAttribute("data-so") : null,
            se: el ? el.getAttribute("data-se") : null,
            closestSo: el && el.closest ? (el.closest("[data-so]") ? el.closest("[data-so]").getAttribute("data-so") : null) : null,
            offset: r.startOffset,
          };
        }
        window.__crfpLog.push({ x, y, r: desc });
        return r;
      };
    }
  });

  // Coordinates of a character inside a rendered text (block idx + needle + offset).
  const charPoint = async (blockIdx, needle, offset) =>
    browser.execute((idx, nd, off) => {
      const blocks = [...document.querySelectorAll(".ls-block")];
      const block = blocks[idx];
      if (!block) return { err: "no block " + idx };
      const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
      let n;
      while ((n = walker.nextNode())) {
        const i = (n.textContent || "").indexOf(nd);
        if (i !== -1) {
          const r = document.createRange();
          r.setStart(n, i + off);
          r.setEnd(n, i + off + 1);
          const b = r.getBoundingClientRect();
          return { x: b.left + Math.min(2, b.width / 2), y: b.top + b.height / 2, found: (n.textContent || "").slice(0, 30) };
        }
      }
      return { err: "text not found: " + nd };
    }, blockIdx, needle, offset);

  const realClick = async (x, y) => {
    await browser.performActions([{
      type: "pointer", id: "mouse", parameters: { pointerType: "mouse" },
      actions: [
        { type: "pointerMove", duration: 0, x: Math.round(x), y: Math.round(y) },
        { type: "pointerDown", button: 0 },
        { type: "pointerUp", button: 0 },
      ],
    }]);
    await browser.releaseActions();
    await sleep(600);
  };

  const probe = async (tag) => {
    const s = await browser.execute(() => {
      const ae = document.activeElement;
      const isEd = ae instanceof HTMLTextAreaElement && ae.classList.contains("block-editor");
      const blocks = [...document.querySelectorAll(".ls-block")];
      const closest = isEd && ae.closest ? ae.closest(".ls-block") : null;
      const editingMain = document.querySelector(".block-main.editing");
      const editingBlock = editingMain ? editingMain.closest(".ls-block") : null;
      return {
        isEditor: isEd,
        sel: isEd ? ae.selectionStart : null,
        val: isEd ? ae.value.slice(0, 40) : null,
        idx: closest ? blocks.indexOf(closest) : -1,
        editingIdx: editingBlock ? blocks.indexOf(editingBlock) : -1,
        aeTag: ae ? ae.tagName : "null",
        crfp: (window.__crfpLog || []).slice(-1)[0] || null,
      };
    });
    console.log(`[${tag}]`, JSON.stringify(s));
    return s;
  };

  console.log("\n=== BUG 1a: click inside bold (line 1) of multiline block 0 ===");
  let p = await charPoint(0, "bold", 2);
  console.log("point:", JSON.stringify(p));
  if (!p.err) { await realClick(p.x, p.y); await probe("bold+2 → expect sel=4"); }
  await browser.keys(["Escape"]); await sleep(400);

  console.log("\n=== BUG 1b: click inside 'second line here' of block 0 ===");
  p = await charPoint(0, "second", 3);
  console.log("point:", JSON.stringify(p));
  if (!p.err) { await realClick(p.x, p.y); await probe("second+3 → expect sel=17"); }
  await browser.keys(["Escape"]); await sleep(400);

  console.log("\n=== BUG 1c: control — click ' rest' on line 1 ===");
  p = await charPoint(0, "rest", 2);
  console.log("point:", JSON.stringify(p));
  if (!p.err) { await realClick(p.x, p.y); await probe("rest+2 → expect sel=11"); }
  await browser.keys(["Escape"]); await sleep(400);

  console.log("\n=== TYPO 1: click 'here' (after -> and --) in block 1 line 1 ===");
  p = await charPoint(1, "here", 2);
  console.log("point:", JSON.stringify(p));
  if (!p.err) { await realClick(p.x, p.y); await probe("typo here+2"); }
  await browser.keys(["Escape"]); await sleep(400);

  console.log("\n=== TYPO 2: click 'dashed' in block 1 line 2 (line contains --) ===");
  p = await charPoint(1, "dashed", 3);
  console.log("point:", JSON.stringify(p));
  if (!p.err) { await realClick(p.x, p.y); await probe("typo dashed+3"); }
  await browser.keys(["Escape"]); await sleep(400);

  console.log("\n=== BUG 2: focus TODO block (idx 1), then click 'plain target below' (idx 2) ===");
  p = await charPoint(2, "focus me", 2);
  console.log("point:", JSON.stringify(p));
  if (!p.err) { await realClick(p.x, p.y); await probe("focused TODO — editor should show 2 lines"); }
  // Now, WITHOUT escaping, click the block below at its CURRENT (shifted) position.
  p = await charPoint(3, "target", 2);
  console.log("point (while TODO editing):", JSON.stringify(p));
  if (!p.err) { await realClick(p.x, p.y); await probe("clicked below → expect editor on idx 2"); }
  await probe("final state");
} finally {
  try { if (browser) await browser.deleteSession(); } catch {}
  td.kill();
  if (xvfb) xvfb.kill();
}
