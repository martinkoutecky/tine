// GH #745: on a long page with images above, scroll a block from the bottom of the viewport to the
// top with the wheel, then make a structural edit (Enter split at start / middle, Backspace merge into
// an image block, Delete merge, Tab / Shift+Tab, multi-block paste): the caret must still be on screen
// afterwards and the view may move only by the least amount that shows it. Before the fix Backspace
// into a tall image block left the caret above the viewport (WebKitGTK; WebView2 centred it instead).
// Run under xvfb-run -s "-screen 0 1920x1080x24" with TINE_APP set. IMAGES=0 drops the images.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import zlib from "node:zlib";
import { setTimeout as delay } from "node:timers/promises";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "tine-gh745-"));
const graph = `${tmp}/graph`;
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${graph}/${dir}`, { recursive: true });

// Opaque solid-colour PNG, `w`x`h`: tiny on disk, real layout height once decoded.
function png(w, h, [r, g, b]) {
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (buf) => { let c = 0xffffffff; for (const x of buf) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const sum = Buffer.alloc(4); sum.writeUInt32BE(crc(body));
    return Buffer.concat([len, body, sum]);
  };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  const row = Buffer.concat([Buffer.from([0]), Buffer.from(Array.from({ length: w }, () => [r, g, b]).flat())]);
  const raw = Buffer.concat(Array.from({ length: h }, () => row));
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", zlib.deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}
const sizes = [[640, 420], [800, 600], [500, 300], [720, 540]];
sizes.forEach(([w, h], i) => fs.writeFileSync(`${graph}/assets/img${i}.png`, png(w, h, [60 + i * 40, 120, 200 - i * 30])));

const ROWS = 400, TARGET = Number(process.env.TARGET_ROW || 120);
const blocks = Array.from({ length: ROWS }, (_, i) => {
  const img = process.env.IMAGES !== "0" && i < TARGET && i % 3 === 0 ? `\n![pic ${i}](../assets/img${(i / 3) % 4}.png)` : "";
  return `- Row ${i} text of the long page${img}\n`;
});
const file = `${graph}/pages/Long.md`;
fs.writeFileSync(file, blocks.join(""));
fs.writeFileSync(`${graph}/pages/Away.md`, "- A small destination\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${tmp}/xdg/${dir}`, { recursive: true });

const port = Number(process.env.E2E_DRIVER_PORT || 4545);
const log = fs.openSync(`${tmp}/driver.log`, "w");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: `${tmp}/xdg/data`, XDG_CONFIG_HOME: `${tmp}/xdg/config`, XDG_CACHE_HOME: `${tmp}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const td = spawn(process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
const results = [];
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt >= 100) throw new Error("Driver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.resolve("target/debug/tine") } } });
  await browser.$(".page-title").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Long");
  await delay(1500);

  // Where (in px from the scroller's top) the row with this text sits; null when its shell is unmounted.
  const rowTop = (needle) => browser.execute((n) => {
    const sc = document.querySelector(".main-content");
    const el = [...document.querySelectorAll(".block-content, textarea.block-editor")]
      .find((e) => (e.value ?? e.textContent).includes(n));
    return { scrollTop: sc.scrollTop, top: el ? Math.round(el.getBoundingClientRect().top - sc.getBoundingClientRect().top) : null,
      height: sc.clientHeight, scrollHeight: sc.scrollHeight };
  }, needle);

  // Caret row (viewport px relative to the scroller) of the focused editor, via an off-screen mirror.
  const caretProbe = () => browser.execute(() => {
    const sc = document.querySelector(".main-content");
    const ta = document.querySelector("textarea.block-editor");
    if (!ta) return null;
    const cs = getComputedStyle(ta);
    const div = document.createElement("div");
    for (const p of ["boxSizing","width","paddingTop","paddingRight","paddingBottom","paddingLeft","borderTopWidth","borderRightWidth","borderBottomWidth","borderLeftWidth","fontFamily","fontSize","fontWeight","fontStyle","letterSpacing","lineHeight","wordSpacing","tabSize","whiteSpace","wordBreak","overflowWrap"])
      div.style[p] = cs[p];
    div.style.position = "absolute"; div.style.left = "-9999px"; div.style.visibility = "hidden";
    document.body.appendChild(div);
    div.textContent = "M";
    const pad = parseFloat(cs.paddingTop) || 0;
    const lh = div.clientHeight - pad - (parseFloat(cs.paddingBottom) || 0);
    const at = ta.selectionEnd;
    div.textContent = "";
    div.appendChild(document.createTextNode(ta.value.slice(0, at)));
    const m = document.createElement("span"); m.textContent = ta.value.slice(at, at + 1) || "\u200b"; div.appendChild(m);
    div.appendChild(document.createTextNode(ta.value.slice(at + 1)));
    const row = Math.max(0, Math.round((m.offsetTop - pad) / lh));
    div.remove();
    const sr = sc.getBoundingClientRect(), tr = ta.getBoundingClientRect();
    const top = tr.top + ta.clientTop + pad + row * lh - sr.top;
    return { scrollTop: sc.scrollTop, caretTop: Math.round(top), caretBottom: Math.round(top + lh), viewHeight: sc.clientHeight, editorTop: Math.round(tr.top - sr.top) };
  });
  const visible = (p) => p && p.caretTop >= -1 && p.caretBottom <= p.viewHeight + 1;

  // Each variant: the row it edits, how the caret is placed, the key, whether the previous row has an image.
  const variants = [
    { name: "enter-middle", row: 115, place: "mid", keys: ["Enter"] },
    { name: "enter-start", row: 112, place: "start", keys: ["Enter"] },
    { name: "backspace-into-image", row: 118, place: "start", keys: ["Backspace"] },
    { name: "delete-merge", row: 104, place: "end", keys: ["Delete"] },
    { name: "tab-then-shift-tab", row: 110, place: "mid", keys: ["Tab", "Shift+Tab"] },
    { name: "paste-multi-block", row: 100, place: "end", paste: "pasted one\npasted two\npasted three" },
  ].filter((v) => !process.env.ONLY || process.env.ONLY.split(",").includes(v.name));

  for (const v of variants) {
    const needle = `Row ${v.row} text`;
    // 1. Bring the row to the BOTTOM of the viewport, letting images/shells settle.
    await browser.execute(() => { document.querySelector(".main-content").scrollTop = 0; });
    await delay(300);
    for (let i = 0; i < 400; i++) {
      const found = await browser.execute((n) => {
        const sc = document.querySelector(".main-content");
        if ([...document.querySelectorAll(".block-content")].some((e) => e.textContent.includes(n))) return true;
        sc.scrollTop += 300; return false;
      }, needle);
      if (found) break;
      await delay(40);
    }
    await delay(700);
    await browser.execute((n) => {
      const sc = document.querySelector(".main-content");
      const el = [...document.querySelectorAll(".block-content")].find((e) => e.textContent.includes(n));
      sc.scrollTop += el.getBoundingClientRect().top - sc.getBoundingClientRect().top - (sc.clientHeight - 120);
    }, needle);
    await delay(700);
    const atBottom = await rowTop(needle);
    // 2. Wheel it up to the TOP.
    const wheelDelta = atBottom.top - 40;
    for (let moved = 0; moved < wheelDelta; moved += 100) {
      await browser.action("wheel").scroll({ x: 600, y: 400, deltaX: 0, deltaY: -Math.min(100, wheelDelta - moved), duration: 0 }).perform()
        .catch(async () => { await browser.execute((d) => { document.querySelector(".main-content").scrollTop -= d; }, Math.min(100, wheelDelta - moved)); });
      await delay(30);
    }
    await delay(900);
    // WebKitGTK has no scroll anchoring: windows mounting above grow from estimate to real height and
    // push the row down. WebView2 (Chromium) anchors; emulate by scrolling the row back to the top.
    for (let settle = 0; settle < 12; settle++) {
      await browser.execute((n) => {
        const sc = document.querySelector(".main-content");
        const el = [...document.querySelectorAll(".block-content")].find((e) => e.textContent.includes(n));
        if (el) sc.scrollTop += el.getBoundingClientRect().top - sc.getBoundingClientRect().top - 40; else sc.scrollTop += 300;
      }, needle);
      await delay(400);
    }
    const atTop = await rowTop(needle);
    // 3. Caret into the row, then the structural edit.
    await browser.execute((n) => {
      const el = [...document.querySelectorAll(".block-content")].find((e) => e.textContent.includes(n));
      el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })); el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true })); el.click();
    }, needle);
    const editor = await browser.$("textarea.block-editor");
    await editor.waitForExist({ timeout: 5000 });
    await delay(300);
    await browser.execute((place) => {
      const t = document.querySelector("textarea.block-editor");
      t.focus({ preventScroll: true });
      const at = place === "start" ? 0 : place === "end" ? t.value.length : 8;
      t.setSelectionRange(at, at);
    }, v.place);
    await delay(200);
    const before = await caretProbe();
    const steps = [];
    const stepKeys = v.paste ? ["paste"] : v.keys;
    for (const key of stepKeys) {
      if (key === "paste") {
        await browser.execute((text) => {
          const t = document.querySelector("textarea.block-editor");
          const dt = new DataTransfer(); dt.setData("text/plain", text);
          t.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
        }, v.paste);
      } else if (key === "Shift+Tab") await browser.keys(["Shift", "Tab"]);
      else await browser.keys(key);
      let probe = null;
      for (const wait of [1, 60, 150, 300, 600, 1200]) { await delay(wait); probe = await caretProbe(); }
      steps.push({ key, probe });
    }
    const last = steps[steps.length - 1].probe;
    const moved = last ? Math.abs(last.scrollTop - before.scrollTop) : null;
    const result = { variant: v.name, images: process.env.IMAGES !== "0", atBottom: atBottom.top, atTop: atTop.top, before, steps, caretVisible: steps.every((s) => visible(s.probe)), scrollMoved: moved };
    results.push(result);
    console.log(JSON.stringify(result));
    await browser.keys("Escape");
    await delay(400);
  }
  const bad = results.filter((r) => !r.caretVisible || r.scrollMoved > 450);
  if (bad.length) throw new Error(`caret lost or view jumped after: ${bad.map((r) => `${r.variant} (visible=${r.caretVisible}, scrollMoved=${r.scrollMoved})`).join("; ")}`);
  console.log(`PASS: every structural edit kept the caret on screen with a minimal scroll. Artifacts: ${tmp}`);
} catch (error) {
  if (browser) { try { await browser.saveScreenshot(`${tmp}/failure.png`); } catch {} }
  console.log(`artifacts: ${tmp}`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
