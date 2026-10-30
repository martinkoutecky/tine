// Geometry probe: rendered block vs textarea editor vs CM6 editor, per construct (CM6-LINEHEIGHT).
// usage: node scripts/shot-cm6-line-height.mjs [out-dir]
// env: BROWSER=chromium|webkitgtk  THEME=light|dark  FONT=<px>  LH=<css length for --tine-typography-line-height>  DPR=1
//   chromium: Playwright Chromium (also writes screenshots). webkitgtk: the system WebKitGTK through
//   WebKitWebDriver + MiniBrowser under Xvfb (what Tine ships on Linux); run it as `xvfb-run -a node ...`.
// Exits 1 when entering the CM6 editor moves anything for a block whose text wraps the same in edit mode
// (box height, the y of the next child block, the first line's top, the line pitch).
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2] || "target/artifacts/cm6-lineheight");
mkdirSync(out, { recursive: true });
const which = process.env.BROWSER || "chromium";
const opts = { theme: process.env.THEME || "light", font: process.env.FONT || "", lh: process.env.LH || "" };
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
const url = `http://localhost:${port}/scripts/fixtures/cm6-line-height.html`;
let close = async () => {}, results;
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 1200) throw new Error("Vite did not start");
    await delay(250);
  }
  if (which === "chromium") {
    const { chromium } = await import("playwright");
    const browser = await chromium.launch({ args: ["--no-sandbox"] });
    close = () => browser.close();
    const page = await browser.newPage({ viewport: { width: 900, height: 700 }, deviceScaleFactor: Number(process.env.DPR || 1) });
    await page.goto(url, { timeout: 120_000 });
    await page.waitForSelector('[data-block-id="plain"] .block-content');
    results = await page.evaluate(o => window.measureAll(o), opts);
    for (const id of ["martin", "h1", "inline", "code", "calc", "deadline", "scheduled", "repeater", "both"]) {
      const shot = kind => page.locator(`[data-block-id="${id}"]`).first().screenshot({ path: `${out}/${which}-${id}-${kind}.png` });
      await page.evaluate(() => window.closeEditor()); await shot("rendered");
      for (const [kind, on] of [["textarea", false], ["cm6", true]]) { await page.evaluate(([i, o]) => window.openEditor(i, o), [id, on]); await shot(kind); }
    }
  } else {
    // W3C WebDriver over HTTP to WebKitWebDriver (no Playwright WebKit: its build needs libs this host lacks).
    const wd = spawn("/usr/bin/WebKitWebDriver", ["--port=4789"], { stdio: "ignore", detached: true });
    close = async () => { try { process.kill(-wd.pid); } catch {} };
    const call = async (method, path, body) => {
      const res = await fetch(`http://127.0.0.1:4789${path}`, { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
      const json = await res.json(); if (json.value?.error) throw new Error(`${json.value.error}: ${json.value.message}`); return json.value;
    };
    for (let a = 0; a < 40; a++) { try { await call("GET", "/status"); break; } catch { await delay(250); } }
    const s = await call("POST", "/session", { capabilities: { alwaysMatch: { browserName: "MiniBrowser",
      "webkitgtk:browserOptions": { binary: "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/MiniBrowser", args: ["--automation"] } } } });
    const sid = s.sessionId;
    await call("POST", `/session/${sid}/window/rect`, { width: 900, height: 700, x: 0, y: 0 });
    await call("POST", `/session/${sid}/url`, { url });
    for (let a = 0; a < 480; a++) { // the dev server transforms on first load
      if (await call("POST", `/session/${sid}/execute/sync`, { script: "return !!document.querySelector('[data-block-id=\"plain\"] .block-content') && !!window.measureAll", args: [] })) break;
      await delay(250);
    }
    await call("POST", `/session/${sid}/timeouts`, { script: 120000 });
    results = await call("POST", `/session/${sid}/execute/async`, { script: "const done = arguments[arguments.length - 1]; window.measureAll(arguments[0]).then(done, e => done({ error: String(e) }));", args: [opts] });
    const ua = await call("POST", `/session/${sid}/execute/sync`, { script: "return navigator.userAgent + ' dpr=' + devicePixelRatio", args: [] });
    console.log("engine:", ua);
  }
  const tag = `${which}-${opts.theme}-fs${opts.font || "def"}-lh${opts.lh || "def"}-dpr${process.env.DPR || 1}`;
  writeFileSync(`${out}/geometry-${tag}.json`, JSON.stringify(results, null, 1));
  const lines = (r, k) => r[k].lines.filter(l => l.h > 10);
  const pitch = ls => { const t = ls.map(l => l.top); return t.length > 1 ? +((t[t.length - 1] - t[0]) / (t.length - 1)).toFixed(2) : null; };
  console.log(tag);
  for (const id of Object.keys(results)) {
    const r = results[id];
    const f = k => r[k].missing ? "MISSING" : `boxH=${r[k].boxH} childY=${r[k].childY} lh=${r[k].lineHeight} pitch=${pitch(lines(r, k))} n=${lines(r, k).length} firstTop=${lines(r, k)[0]?.top ?? "-"}`;
    console.log(id.padEnd(9), "R:", f("rendered")); console.log("".padEnd(9), "T:", f("textarea")); console.log("".padEnd(9), "C:", f("cm6"));
  }
  const bad = [];
  for (const id of ["plain", "martin", "cjk", "emoji", "multiline", "empty", "deadline", "scheduled", "repeater", "both"]) {
    const r = results[id]; const c = r.cm6, d = r.rendered;
    if (c.missing || d.missing) { bad.push(`${id}: missing editor`); continue; }
    const cl = lines(r, "cm6"), dl = lines(r, "rendered");
    // A different line COUNT means the text wrapped differently in edit mode (e.g. the editable emoji face is wider
    // than the rendered one, in the textarea too); then only the pitch and the first line are comparable.
    const sameWrap = cl.length === dl.length;
    if (sameWrap && Math.abs(c.boxH - d.boxH) > 0.5) bad.push(`${id}: box height ${c.boxH} vs rendered ${d.boxH}`);
    if (sameWrap && Math.abs(c.childY - d.childY) > 0.5) bad.push(`${id}: next block y ${c.childY} vs rendered ${d.childY}`);
    if (!sameWrap) console.log(`note ${id}: wraps to ${cl.length} lines vs rendered ${dl.length} (textarea box ${r.textarea.boxH})`);
    if (cl.length && dl.length) {
      if (Math.abs(cl[0].top - dl[0].top) > 0.5) bad.push(`${id}: first line top ${cl[0].top} vs rendered ${dl[0].top}`);
      if (Math.abs(pitch(cl) - pitch(dl)) > 0.1) bad.push(`${id}: line pitch ${pitch(cl)} vs rendered ${pitch(dl)}`);
    }
  }
  console.log(bad.length ? "FAIL\n" + bad.join("\n") : "PASS: CM6 edit mode moves nothing");
  if (bad.length) process.exitCode = 1;
} finally {
  await close();
  try { process.kill(-server.pid); } catch {}
}
