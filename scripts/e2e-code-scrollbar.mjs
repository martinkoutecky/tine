// GH #787: the horizontal scrollbar under a long-line code block is easy to grab: a real
// pointer press on the bar scrolls the card and never opens the block's editor.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const probe = process.env.SCROLLBAR_PROBE === "1"; // report every pressed row instead of asserting
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/code-scrollbar-787"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const long = Array.from({ length: 60 }, (_, i) => `token${String(i).padStart(2, "0")}`).join(" ");
const body = "- Before\n- ```js\n  const first = 1;\n  const long = \"" + long + "\";\n  const last = 3;\n  ```\n- After\n";
fs.writeFileSync(path.join(graph, "pages", "Code Scroll.md"), body);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4890);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "A real pointer press on a long-line code block's horizontal scrollbar, then a drag, scrolls the card; the block stays rendered (no editor opens); a press on the code text still opens the editor" };
let browser;
let stage = "startup";
const shot = (name) => { try { execFileSync("import", ["-window", "root", path.join(out, name)], { env }); } catch {} };
const card = () => browser.execute(() => {
  const el = document.querySelector("pre.code-block");
  if (!el) return null;
  const r = el.getBoundingClientRect();
  const cs = getComputedStyle(el);
  return { left: r.left, top: r.top, width: r.width, height: r.height, scrollLeft: el.scrollLeft,
    scrollWidth: el.scrollWidth, clientWidth: el.clientWidth, clientHeight: el.clientHeight, offsetHeight: el.offsetHeight,
    borderBottom: parseFloat(cs.borderBottomWidth) || 0, paddingBottom: parseFloat(cs.paddingBottom) || 0,
    overlay: el.offsetHeight - el.clientHeight - (parseFloat(cs.borderTopWidth) || 0) - (parseFloat(cs.borderBottomWidth) || 0) <= 0.5 };
});
const editing = () => browser.execute(() => !!document.querySelector(".main-content textarea.block-editor"));
const reset = async () => {
  await browser.keys(["Escape"]);
  await browser.waitUntil(async () => !(await editing()), { timeout: 5000 });
  await browser.execute(() => { document.querySelector("pre.code-block").scrollLeft = 0; });
};
// Press at (x, y) viewport coordinates, drag `dx` right, release. Returns what changed.
const pressDrag = async (x, y, dx) => {
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, x: Math.round(x), y: Math.round(y) },
    { type: "pointerDown", button: 0 },
    { type: "pointerMove", duration: 150, x: Math.round(x + dx / 2), y: Math.round(y) },
    { type: "pointerMove", duration: 150, x: Math.round(x + dx), y: Math.round(y) },
    { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  await sleep(250);
  const after = await card();
  return { scrollLeft: after?.scrollLeft ?? null, editing: await editing() };
};
try {
  await sleep(1500);
  for (let attempt = 0; ; attempt++) {
    try {
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "code-scrollbar") });
      break;
    } catch (error) {
      if (!String(error).includes("Maximum number of active sessions") || attempt >= 11) throw error;
      console.log("Another native journey owns WebKit; waiting 10 seconds before retry.");
      await sleep(10000);
    }
  }
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Code Scroll");
  await browser.waitUntil(async () => { const c = await card(); return !!c && c.scrollWidth > c.clientWidth + 50; },
    { timeout: 15000, timeoutMsg: "the long-line code card did not render with horizontal overflow" });
  const geometry = await card();
  proof.geometry = geometry;
  shot("card.png");
  const thumbWidth = Math.max(20, geometry.clientWidth * geometry.clientWidth / geometry.scrollWidth);
  // The thumb starts at the card's left inner edge; aim at its middle.
  const x = geometry.left + 1 + thumbWidth / 2;
  const bottom = geometry.top + geometry.height; // border-box bottom edge
  if (probe) {
    const rows = [];
    for (let up = 1; up <= 16; up++) {
      await reset();
      const hit = await browser.execute((px, py) => { const e = document.elementFromPoint(px, py); return e ? `${e.tagName}.${e.className}` : null; }, x, bottom - up);
      rows.push({ pixelsAboveBottomEdge: up, hit, ...(await pressDrag(x, bottom - up, 80)) });
    }
    proof.probe = rows; proof.outcome = "PROBE";
    fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
    console.log(JSON.stringify({ geometry, rows }, null, 1));
  } else {
    // The visible bar is the bottom strip of the card: every row inside the bottom 6 px
    // (inside the border) must drag-scroll the card and leave it rendered.
    stage = "press and drag the scrollbar";
    const bad = [];
    for (const up of [3, 6, 9, 12]) {
      await reset();
      const r = await pressDrag(x, bottom - up, 80);
      if (!(r.scrollLeft > 10) || r.editing) bad.push({ up, ...r });
    }
    if (bad.length) throw Error(`pressing the bar failed: ${JSON.stringify(bad)} geometry ${JSON.stringify(geometry)}`);
    stage = "the code text still opens the editor";
    await reset();
    await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, x: Math.round(geometry.left + 80), y: Math.round(geometry.top + 40) },
      { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 } ] }]);
    await browser.releaseActions();
    await browser.waitUntil(editing, { timeout: 5000, timeoutMsg: "a click on the code text no longer opens the editor" });
    shot("editing.png");
    // Themes: the bar's thumb follows --scrollbar-thumb in every built-in palette (screenshots to look at).
    await reset();
    for (const [name, scheme, theme] of [["light", "light", ""], ["dark", "dark", ""], ["soft-light", "light", "soft"], ["soft-dark", "dark", "soft"]]) {
      await browser.execute((scheme, theme) => { document.documentElement.setAttribute("data-theme", scheme); window.__tineApplyTheme?.(theme); }, scheme, theme);
      await sleep(300);
      await browser.execute(() => { const el = document.querySelector("pre.code-block"); el.scrollLeft = 300; });
      await sleep(200);
      shot(`theme-${name}.png`);
    }
    proof.outcome = "PASS";
  }
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`${proof.outcome} GH #787: ${out}`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "A pointer press-and-drag on the code card's horizontal scrollbar scrolls it without opening the editor",
    observed: String(error.stack), classification: "ambiguous; inspect geometry, driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
