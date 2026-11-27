// Cursor probe (UI-CM6-LINK-CURSOR): inside a block edited in CodeMirror, a pointer over a drawn link, tag or block ref only
// reveals the source (src/editor/cm6PreviewWidget.tsx), so every element of a preview shows the text cursor; in the
// rendered block a click follows the link, so the same element keeps the hand.
// usage: xvfb-run -a node scripts/shot-cm6-link-cursor.mjs [out-dir]
//   The system WebKitGTK (what Tine ships on Linux) through WebKitWebDriver + MiniBrowser, on the cm6-line-height fixture.
// Exits 1 when an element inside an editing preview computes a cursor other than `text` (the code block's language button:
// other than `pointer`), when a link kind is not drawn as a preview, or when the rendered link loses its cursor.
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
// Runs in the page: the rendered link's cursor, then (the caret at the end, away from the construct, so the construct is
// drawn as a preview) the cursor of every element of the editing previews.
const PROBE = `const [cases, done] = [arguments[0], arguments[arguments.length - 1]];
(async () => {
  const rows = [];
  await window.openEditor("plain", true); // the first CodeMirror mount loads the editor's chunk
  for (const [id, [link, hand]] of Object.entries(cases)) {
    await window.closeEditor();
    const el = document.querySelector('[data-block-id="' + id + '"] .block-content')?.querySelector(link);
    const rendered = el ? getComputedStyle(el).cursor : null;
    await window.openEditor(id, true, 1e9);
    const previews = Array.from(document.querySelectorAll('[data-block-id="' + id + '"] .cm6-preview'));
    const els = previews.flatMap(p => [p, ...p.querySelectorAll("*")]).filter(e => !e.closest("button.code-language"));
    const wrong = els.map(e => e.tagName.toLowerCase() + "." + String(e.className).replace(/\\s+/g, ".") + "=" + getComputedStyle(e).cursor)
      .filter(s => !s.endsWith("=text"));
    const drawn = previews.some(p => p.querySelector(link));
    const controls = previews.flatMap(p => [...p.querySelectorAll("button.code-language")]).map(e => getComputedStyle(e).cursor);
    rows.push({ id, link, hand, rendered, drawn, wrong, controls, ...(drawn ? {} : { html: document.querySelector('[data-block-id="' + id + '"] .cm-content')?.outerHTML.slice(0, 600) }) });
  }
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
  rows = await call("POST", `/session/${sid}/execute/async`, { script: PROBE, args: [LINKS] });
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
  console.log(r.id.padEnd(11), r.link.padEnd(13), "rendered:", String(r.rendered).padEnd(8), "drawn:", r.drawn, "wrong:", r.wrong.join(" ") || "-", "controls:", r.controls.join(" ") || "-");
  if (r.rendered !== r.hand) bad.push(`${r.id}: rendered ${r.link} cursor ${r.rendered}, expected ${r.hand}`);
  if (!r.drawn) bad.push(`${r.id}: ${r.link} not drawn in an editing preview: ${r.html}`);
  for (const c of r.controls) if (c !== "pointer") bad.push(`${r.id}: editing preview language button cursor ${c}, expected pointer`);
  for (const w of r.wrong) bad.push(`${r.id}: editing preview ${w}, expected text`);
}
console.log(bad.length ? "FAIL\n" + bad.join("\n") : "PASS: links in an editing preview show the text cursor, rendered links the hand");
process.exit(bad.length ? 1 : 0);
