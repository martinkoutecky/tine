// Cursor probe (UI-CM6-LINK-CURSOR): inside a block edited in CodeMirror, a pointer over a drawn link, tag or block ref only
// reveals the source (src/editor/cm6PreviewWidget.tsx), so every element of a preview shows the text cursor; in the
// rendered block a click follows the link, so the same element keeps the hand. An element that takes its own mouse press
// before the reveal (the code block's language button, the image and video resize grips, a writable table's column grip)
// keeps its own cursor there, in left-to-right and right-to-left documents.
// usage: xvfb-run -a node scripts/shot-cm6-link-cursor.mjs [out-dir]
//   The system WebKitGTK (what Tine ships on Linux) through WebKitWebDriver + MiniBrowser, on the cm6-line-height fixture.
// Exits 1 when an element inside an editing preview computes a cursor other than `text` (an own-press element: other than
// its own), when a link kind or an own-press element is not drawn in an editing preview, or when the rendered link loses
// its cursor.
import { spawn } from "node:child_process";
import { closeSync, mkdirSync, openSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2] || "target/artifacts/cm6-link-cursor");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const wdPort = 20000 + Math.floor(Math.random() * 20000);
const serverLog = openSync(`${out}/vite.log`, "w");
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: ["ignore", serverLog, serverLog], detached: true });
closeSync(serverLog);
// Fixture case -> the link element the view draws (every link kind the preview widget draws inline) and its cursor in the
// rendered block: the hand (`pointer`; `auto` over an `a[href]`) where a click follows it. The fixture has no backend, so
// its block ref is dangling (`.block-ref-missing`, no hand), and a footnote reference has no hand in the view either.
const LINKS = {
  i_page: [".page-ref", "pointer"], i_tag: [".tag", "pointer"], i_btag: [".tag", "pointer"], i_aliasref: [".page-ref", "pointer"],
  i_bref: [".block-ref", "default"], i_link: ["a[href]", "auto"], i_url: ["a[href]", "auto"], i_fn: [".footnote-ref", "default"],
  todotags: [".tag", "pointer"],
};
// Own-press elements -> their cursor in a left-to-right and a right-to-left document (src/styles/app.css).
const OWN = { "button.code-language": ["pointer", "pointer"], ".img-resize-grip": ["nwse-resize", "nesw-resize"],
  ".sheet-table-column-resize-handle": ["col-resize", "col-resize"] };
// Fixture case -> the own-press element its editing preview must draw.
const OWNED = { c_image: ".img-resize-grip", c_video: ".media-resize-grip", c_vurl: ".media-resize-grip" };
// Runs in the page: the rendered link's cursor, then (the caret at the end, away from the construct, so the construct is
// drawn as a preview) the cursor of every element of the editing previews; then the media grip cases in both directions
// and a seeded preview holding a writable table column grip and a code-language button.
const PROBE = `const [cases, owned, own, done] = [arguments[0], arguments[1], arguments[2], arguments[arguments.length - 1]];
(async () => {
  const rows = [];
  const label = e => e.tagName.toLowerCase() + "." + String(e.className).replace(/\\s+/g, ".");
  const scan = (previews, rtl) => {
    const all = previews.flatMap(p => [p, ...p.querySelectorAll("*")]);
    const ownSel = Object.keys(own).join(",");
    const wrong = all.filter(e => !e.closest(ownSel)).map(e => label(e) + "=" + getComputedStyle(e).cursor).filter(s => !s.endsWith("=text"));
    const owns = all.filter(e => e.matches(ownSel)).map(e => {
      const sel = Object.keys(own).find(k => e.matches(k)), cursor = getComputedStyle(e).cursor;
      return { sel, cursor, ok: cursor === own[sel][rtl ? 1 : 0] };
    });
    return { wrong, owns };
  };
  // This host's WebKitGTK decodes no video (MediaError 4 for mp4 and webm), and a video that fails is replaced by a
  // chip without a grip. Keep media elements from loading: the embed, its wrapper and its grip render as shipped.
  const setAttribute = Element.prototype.setAttribute;
  HTMLMediaElement.prototype.setAttribute = function (name, value) { return name === "src" ? undefined : setAttribute.call(this, name, value); };
  Object.defineProperty(HTMLMediaElement.prototype, "src", { configurable: true, get() { return ""; }, set() {} });
  await window.openEditor("plain", true); // the first CodeMirror mount loads the editor's chunk
  for (const [id, [link, hand]] of Object.entries(cases)) {
    await window.closeEditor();
    const el = document.querySelector('[data-block-id="' + id + '"] .block-content')?.querySelector(link);
    const rendered = el ? getComputedStyle(el).cursor : null;
    await window.openEditor(id, true, 1e9);
    const previews = Array.from(document.querySelectorAll('[data-block-id="' + id + '"] .cm6-preview'));
    const drawn = previews.some(p => p.querySelector(link));
    rows.push({ id, link, hand, rendered, drawn, ...scan(previews, false), ...(drawn ? {} : { html: document.querySelector('[data-block-id="' + id + '"] .cm-content')?.outerHTML.slice(0, 600) }) });
  }
  for (const dir of ["ltr", "rtl"]) {
    document.documentElement.dir = dir;
    for (const [id, need] of Object.entries(owned)) {
      await window.openEditor(id, true, 1e9);
      const previews = Array.from(document.querySelectorAll('[data-block-id="' + id + '"] .cm6-preview'));
      const drawn = previews.some(p => p.querySelector(need));
      rows.push({ id: id + ":" + dir, link: need, own: true, drawn, ...scan(previews, dir === "rtl"), html: document.querySelector('[data-block-id="' + id + '"] .cm-content')?.outerHTML.slice(0, 6000) });
    }
    await window.closeEditor();
    const host = window.mountOwnPressHost();
    rows.push({ id: "seeded:" + dir, link: "column grip, language button", own: true, drawn: !!host.querySelector(".sheet-table-column-resize-handle") && !!host.querySelector("button.code-language"), ...scan([host], dir === "rtl"), html: host.outerHTML.slice(0, 600) });
    host.remove();
  }
  document.documentElement.dir = "";
  await window.closeEditor();
  return rows;
})().then(done, e => done({ error: String(e) }));`;
let rows, close = async () => {};
try {
  for (let attempt = 0; ; attempt++) {
    if (server.exitCode !== null) throw new Error(`Vite exited ${server.exitCode}; see ${out}/vite.log`);
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 1200) throw new Error("Vite did not start");
    await delay(250);
  }
  const wd = spawn("/usr/bin/WebKitWebDriver", [`--port=${wdPort}`], { stdio: "ignore", detached: true });
  close = async () => { try { process.kill(-wd.pid); } catch {} };
  const call = async (method, path, body) => {
    const res = await fetch(`http://127.0.0.1:${wdPort}${path}`, { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
    const json = await res.json(); if (json.value?.error) throw new Error(`${json.value.error}: ${json.value.message}`); return json.value;
  };
  for (let a = 0; a < 40; a++) { try { await call("GET", "/status"); break; } catch { await delay(250); } }
  const { sessionId: sid } = await call("POST", "/session", { capabilities: { alwaysMatch: { browserName: "MiniBrowser",
    "webkitgtk:browserOptions": { binary: "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/MiniBrowser", args: ["--automation"] } } } });
  await call("POST", `/session/${sid}/window/rect`, { width: 900, height: 700, x: 0, y: 0 });
  await call("POST", `/session/${sid}/url`, { url: `http://localhost:${port}/scripts/fixtures/cm6-line-height.html` });
  for (let a = 0; a < 480; a++) { // the dev server transforms on first load
    if (await call("POST", `/session/${sid}/execute/sync`, { script: "return !!document.querySelector('[data-block-id=\"plain\"] .block-content') && !!window.openEditor", args: [] })) break;
    await delay(250);
  }
  await call("POST", `/session/${sid}/timeouts`, { script: 240000 });
  rows = await call("POST", `/session/${sid}/execute/async`, { script: PROBE, args: [LINKS, OWNED, OWN] });
  console.log("engine:", await call("POST", `/session/${sid}/execute/sync`, { script: "return navigator.userAgent", args: [] }));
  await call("DELETE", `/session/${sid}`);
} finally {
  await close();
  try { process.kill(-server.pid); } catch {}
}
if (!Array.isArray(rows)) throw new Error(`cursor probe failed: ${JSON.stringify(rows)}`);
writeFileSync(`${out}/results.json`, JSON.stringify(rows, null, 2));
const bad = [];
for (const r of rows) {
  const owns = r.owns.map(o => o.sel + "=" + o.cursor).join(" ") || "-";
  console.log(r.id.padEnd(16), r.link.padEnd(13), "rendered:", String(r.rendered ?? "-").padEnd(8), "drawn:", r.drawn, "wrong:", r.wrong.join(" ") || "-", "own:", owns);
  if (!r.own && r.rendered !== r.hand) bad.push(`${r.id}: rendered ${r.link} cursor ${r.rendered}, expected ${r.hand}`);
  if (!r.drawn) bad.push(`${r.id}: ${r.link} not drawn in an editing preview: ${r.html}`);
  for (const o of r.owns) if (!o.ok) bad.push(`${r.id}: editing preview ${o.sel} cursor ${o.cursor}, expected its own`);
  for (const w of r.wrong) bad.push(`${r.id}: editing preview ${w}, expected text`);
}
console.log(bad.length ? "FAIL\n" + bad.join("\n") : "PASS: links in an editing preview show the text cursor, rendered links the hand, own-press elements their own cursor");
process.exit(bad.length ? 1 : 0);
