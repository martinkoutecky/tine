// Geometry probe: rendered block vs textarea editor vs CM6 editor, per construct (CM6-LINEHEIGHT).
// usage: node scripts/shot-cm6-line-height.mjs [out-dir]
// env: BROWSER=chromium|webkitgtk  THEME=light|dark  FONT=<px>  LH=<css length for --tine-typography-line-height>  DPR=1
//   chromium: Playwright Chromium (also writes screenshots). webkitgtk: the system WebKitGTK through
//   WebKitWebDriver + MiniBrowser under Xvfb (what Tine ships on Linux); run it as `xvfb-run -a node ...`.
// Exits 1 when entering the CM6 editor (caret at the start or at the end) moves anything for a gated construct whose text
// wraps the same in edit mode (box height, the y of the next child block, the first line's top, the line pitch), or when
// the textarea editor of a single-line heading differs from the rendered heading. Every other construct is only tabled.
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
    for (const id of ["todo", "todop", "donep", "canceled", "todotags", "martin", "h1", "mdh1body", "inline", "code", "calc", "deadline", "scheduled", "repeater", "both", "sameline", "list", "quote", "props", "table", "image", "math", "embed", "quote2", "table2", "math2", "image2"]) {
      const shot = kind => page.locator(`[data-block-id="${id}"]`).first().screenshot({ path: `${out}/${which}-${id}-${kind}.png` });
      await page.evaluate(() => window.closeEditor()); await shot("rendered");
      for (const [kind, on, caret] of [["textarea", false, 0], ["cm6", true, 0], ["cm6end", true, 1e9]]) { await page.evaluate(([i, o, c]) => window.openEditor(i, o, c), [id, on, caret]); await shot(kind); }
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
  const table = ["id\tR.childY\tT.childY\tC.childY\tE.childY\tdC\tdE\tdT"];
  for (const id of Object.keys(results)) {
    const r = results[id];
    const f = k => r[k].missing ? "MISSING" : `boxH=${r[k].boxH} childY=${r[k].childY} lh=${r[k].lineHeight} pitch=${pitch(lines(r, k))} n=${lines(r, k).length} firstTop=${lines(r, k)[0]?.top ?? "-"}`;
    console.log(id.padEnd(9), "R:", f("rendered")); console.log("".padEnd(9), "T:", f("textarea")); console.log("".padEnd(9), "C:", f("cm6")); console.log("".padEnd(9), "E:", f("cm6end"));
    const y = k => r[k].missing ? NaN : r[k].childY;
    table.push([id, y("rendered"), y("textarea"), y("cm6"), y("cm6end"), +(y("cm6") - y("rendered")).toFixed(2), +(y("cm6end") - y("rendered")).toFixed(2), +(y("textarea") - y("rendered")).toFixed(2)].join("\t"));
  }
  writeFileSync(`${out}/table-${tag}.tsv`, table.join("\n") + "\n");
  // What must hold. `lineChecks`: the CM6 text lines sit where the rendered ones do (only meaningful when both are plain
  // text lines); `boxOnly`: the construct is a card/widget, so only the block height and the next block's y are compared.
  // `E` is the same with the caret at the end of the text (constructs concealed unless the caret is on them).
  const boxOnly = new Set(["code", "calc", "props2"]);
  const cm6Gated = ["plain", "martin", "cjk", "emoji", "multiline", "empty", "deadline", "scheduled", "repeater", "both", "sameline",
    "h1", "h2", "h3", "h4", "h5", "h6", "mdh1", "mdh1body", "h2body", "hauto", "code", "calc",
    "inline", "list", "quote2", "table2", "math2", "image2", "olist2", "props2", "hr"];
  const cm6EndGated = ["plain", "multiline", "mdh1body", "h2body", "code", "calc"]; // planning lines reveal under a caret at the end, by design
  const textareaGated = ["h1", "mdh1", "hauto"]; // uniline headings: OG keeps the heading size (h2..h6: line-height differs, see the receipt)
  const bad = [];
  const compare = (id, kind, label) => {
    const r = results[id]; const c = r[kind], d = r.rendered;
    if (c.missing || d.missing) { bad.push(`${id}: missing editor (${label})`); return; }
    const cl = lines(r, kind), dl = lines(r, "rendered");
    // A different line COUNT means the text wrapped differently in edit mode (e.g. the editable emoji face is wider
    // than the rendered one, in the textarea too); then only the pitch and the first line are comparable.
    const sameWrap = boxOnly.has(id) || cl.length === dl.length;
    if (sameWrap && Math.abs(c.boxH - d.boxH) > 0.5 && !boxOnly.has(id)) bad.push(`${id}: ${label} box height ${c.boxH} vs rendered ${d.boxH}`);
    if (sameWrap && Math.abs(c.childY - d.childY) > 0.5) bad.push(`${id}: ${label} next block y ${c.childY} vs rendered ${d.childY}`);
    if (!sameWrap) console.log(`note ${id}: ${label} wraps to ${cl.length} lines vs rendered ${dl.length} (textarea box ${r.textarea.boxH})`);
    if (!boxOnly.has(id) && cl.length && dl.length) {
      if (Math.abs(cl[0].top - dl[0].top) > 0.5) bad.push(`${id}: ${label} first line top ${cl[0].top} vs rendered ${dl[0].top}`);
      // A text mixed with inline widgets (math, chips) has no single pitch once it wraps differently.
      if ((sameWrap || id !== "inline") && Math.abs(pitch(cl) - pitch(dl)) > 0.1) bad.push(`${id}: ${label} line pitch ${pitch(cl)} vs rendered ${pitch(dl)}`);
    }
  };
  // A planning line is chips, never `<date>` text, in the view and in the editor (CM6 with the caret away from it).
  for (const id of ["deadline", "scheduled", "repeater", "both", "sameline"]) {
    for (const kind of ["rendered", "cm6"]) {
      if (results[id][kind].missing) continue;
      if (/<\d{4}-/.test(results[id][kind].val)) bad.push(`${id}: ${kind} body shows a stray <date>: ${JSON.stringify(results[id][kind].val)}`);
    }
  }
  // The leading chip group (checkbox, marker chip, priority chip) looks and sits the same in the view and in CM6 with the caret
  // away from it (CM6-TASKMARKER); the caret on the marker shows its source by design, then only the other chips are compared.
  const sameChips = (id, kind, label, skip) => {
    const a = results[id].rendered.chips ?? [], b = results[id][kind].chips ?? [];
    const keep = (l) => skip ? l.filter(c => !skip.includes(c.cls)) : l;
    const A = keep(a), B = keep(b);
    if (A.length !== B.length || A.some((c, i) => c.cls !== B[i].cls)) { bad.push(`${id}: ${label} chips ${B.map(c => c.cls)} vs rendered ${A.map(c => c.cls)}`); return; }
    A.forEach((c, i) => {
      const d = B[i];
      for (const k of ["text", "border", "radius", "bg", "color", "fontSize", "margin", "pad", "va"]) if (c[k] !== d[k]) bad.push(`${id}: ${label} ${c.cls} ${k} ${JSON.stringify(d[k])} vs rendered ${JSON.stringify(c[k])}`);
      for (const k of ["w", "h"]) if (Math.abs(c[k] - d[k]) > 0.5) bad.push(`${id}: ${label} ${c.cls} ${k} ${d[k]} vs rendered ${c[k]}`);
      if (!skip) for (const k of ["x", "y"]) if (Math.abs(c[k] - d[k]) > 0.5) bad.push(`${id}: ${label} ${c.cls} ${k} ${d[k]} vs rendered ${c[k]}`);
    });
  };
  const markerIds = ["todo", "doing", "now", "later", "done", "waiting", "canceled", "todop", "doingp", "donep", "todotags", "priority"];
  for (const id of markerIds) { sameChips(id, "cm6end", "CM6 (caret at end)"); compare(id, "cm6end", "CM6 (caret at end)"); }
  // A caret at 0 sits on the marker: its source is shown (D-20261025-CR), so only a priority chip after it is compared (by look, not place).
  for (const id of ["todop", "doingp", "donep"]) sameChips(id, "cm6", "CM6 (caret on the marker)", ["block-task-checkbox", "block-marker"]);
  // An empty block in CM6 shows a caret (CM6-EMPTYCARET): WebKitGTK paints no native one in an empty line, so CM draws its own, as tall
  // as text, on the line's left edge, and hides the native one meanwhile. A block with text never draws one.
  for (const kind of ["cm6", "cm6end"]) {
    const d = results.empty[kind].drawnCaret;
    if (!d) bad.push(`empty: ${kind} draws no caret of its own (an empty block would show none in WebKitGTK)`);
    else {
      if (!d.shown) bad.push(`empty: ${kind} caret layer is not displayed`);
      if (d.h < 12) bad.push(`empty: ${kind} caret height ${d.h}`);
      if (Math.abs(d.x) > 2) bad.push(`empty: ${kind} caret x ${d.x} from the line's left edge`);
      if (!/rgba?\(0, 0, 0, 0\)|transparent/.test(d.native)) bad.push(`empty: ${kind} native caret is not hidden (${d.native}) while the drawn one shows`);
    }
    for (const id of ["plain", "multiline", "todo"]) if (results[id][kind].drawnCaret) bad.push(`${id}: ${kind} draws an empty-block caret in a block with text`);
  }
  // Every inline element (tags, page refs, links, code, highlights, math, ...) sits and looks the same in CM6 with the caret away.
  for (const id of Object.keys(results).filter(i => i.startsWith("i_"))) {
    compare(id, "cm6end", "CM6 (caret at end)");
    const A = results[id].rendered.inl ?? [], B = results[id].cm6end.inl ?? [];
    if (A.length !== B.length) { bad.push(`${id}: CM6 inline elements ${B.map(e => e.el)} vs rendered ${A.map(e => e.el)}`); continue; }
    A.forEach((a, i) => { const b = B[i];
      for (const k of ["el", "text", "color", "bg", "fontSize", "fontWeight", "fontStyle", "deco", "border", "radius", "pad", "family", "display"]) if (a[k] !== b[k]) bad.push(`${id}: CM6 ${a.el} ${k} ${JSON.stringify(b[k])} vs rendered ${JSON.stringify(a[k])}`);
      for (const k of ["x", "y", "w", "h"]) if (Math.abs(a[k] - b[k]) > 0.5) bad.push(`${id}: CM6 ${a.el} ${k} ${b[k]} vs rendered ${a[k]}`); });
  }
  for (const id of cm6Gated) compare(id, "cm6", "CM6");
  for (const id of cm6EndGated) compare(id, "cm6end", "CM6 (caret at end)");
  // The textarea heading class is `line-height: 1.3` while the view's heading line sits on the block's strut line: they
  // agree at the default line height only (a customised `--tine-typography-line-height` changes the view's line box and
  // not the textarea's; follow-up in the receipt). WebKit also rounds the textarea's fractional line height down per
  // row, so the allowance is 1px per rendered line. A `# ` heading shows its marker in both editors: when that wraps
  // the line differently, the textarea box is not comparable (h1 carries the same check without the marker).
  for (const id of textareaGated) {
    const r = results[id];
    if (opts.lh) { console.log(`note ${id}: textarea heading check skipped with a customised line height`); continue; }
    if (r.textarea.missing) { bad.push(`${id}: missing textarea`); continue; }
    if (id === "mdh1" && lines(r, "cm6").length !== lines(r, "rendered").length) { console.log(`note ${id}: textarea heading check skipped, the visible marker wraps the line differently`); continue; }
    const tol = Math.max(0.5, lines(r, "rendered").length);
    if (Math.abs(r.textarea.boxH - r.rendered.boxH) > tol) bad.push(`${id}: textarea box height ${r.textarea.boxH} vs rendered ${r.rendered.boxH}`);
    if (Math.abs(r.textarea.childY - r.rendered.childY) > tol) bad.push(`${id}: textarea next block y ${r.textarea.childY} vs rendered ${r.rendered.childY}`);
  }
  console.log(bad.length ? "FAIL\n" + bad.join("\n") : "PASS: editing moves nothing for the gated constructs");
  if (bad.length) process.exitCode = 1;
} finally {
  await close();
  try { process.kill(-server.pid); } catch {}
}
