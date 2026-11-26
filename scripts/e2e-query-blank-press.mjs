// Native journey for GH #1000: a plain click inside a query's results (between
// two page groups) opens no editor, and the page
// does not jump. The press used to open the block that holds the query, whose
// text replaced the results; the page then scrolled to its caret.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { remote } from "webdriverio";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/gh1000");
const app = process.env.TINE_APP || path.join(root, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const driverPort = +(process.env.E2E_DRIVER_PORT || 4712);
const nativePort = +(process.env.E2E_NATIVE_PORT || 4713);
const run = fs.mkdtempSync(path.join(os.tmpdir(), "tine-gh1000-"));
fs.mkdirSync(out, { recursive: true });
const graph = path.join(run, "graph");
for (const dir of ["logseq", "pages", "journals"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
// 8 source pages x 5 results = 40 results in 8 page groups.
for (let p = 1; p <= 8; p++) {
  fs.writeFileSync(path.join(graph, `pages/Source ${p}.md`),
    Array.from({ length: 5 }, (_, i) => `- TODO Result ${p}.${i + 1}\n`).join(""));
}
const hostFile = path.join(graph, "pages/Press host.md");
fs.writeFileSync(hostFile, "- Before the query\n- {{query (task TODO)}}\n- After the query\n");
const hostText = fs.readFileSync(hostFile, "utf8");

const fixture = accountHomeFixture(run, out, {
  ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"),
  APPDATA: path.join(run, "appdata"), LOCALAPPDATA: path.join(run, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", GDK_BACKEND: "x11",
});
const launcher = await startWebdriverApplication(app, fixture.env, nativePort, "gh1000");
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(driverPort, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: launcher.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });

/** Scroll the query's middle into view, then name a point of the given kind inside its results. */
function aimAt(kind) {
  const scroller = document.querySelector(".main-content");
  const region = scroller.querySelector(".query-results-region");
  const groups = [...region.querySelectorAll(":scope .query-group")];
  const middle = groups[Math.floor(groups.length / 2)];
  scroller.scrollTop += middle.getBoundingClientRect().top - scroller.getBoundingClientRect().top - scroller.clientHeight / 3;
  // The query sits in its host block's own .block-content: only an ancestor inside the results counts.
  const inside = (el, selector) => { const found = el.closest(selector); return !!found && region.contains(found); };
  let x, y;
  // In the native layout a result row's text fills the row, so the blank
  // space a press can reach natively is the gap between two page groups; a
  // press beside a row's text is covered by QueryResultBlankPress.test.tsx.
  const above = groups[groups.indexOf(middle) - 1].getBoundingClientRect();
  const below = middle.getBoundingClientRect();
  x = below.left + below.width / 2;
  y = Math.round((above.bottom + below.top) / 2);
  const hit = document.elementFromPoint(x, y);
  return { x: Math.round(x), y: Math.round(y), top: scroller.scrollTop,
    inResults: !!hit && region.contains(hit), onText: !!hit && inside(hit, ".block-content"),
    hit: hit ? `${hit.tagName}.${hit.className}` : null };
}
function afterPress(done) {
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const scroller = document.querySelector(".main-content");
    done({ top: scroller.scrollTop,
      editor: !!scroller.querySelector("textarea.block-editor, [data-block-editor]"),
      rows: scroller.querySelectorAll(".query-results-region .ls-block").length });
  }));
}

let browser;
let phase = "driver";
try {
  const deadline = Date.now() + 20_000;
  while ((await fetch(`http://127.0.0.1:${driverPort}/status`).catch(() => null))?.ok !== true) {
    if (driver.exitCode !== null || Date.now() >= deadline) throw new Error("WebDriver did not start");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/",
    logLevel: "error", connectionRetryCount: 0, capabilities: tauriCapabilities(app, "gh1000", process.platform, launcher.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setTimeout({ script: 20_000 });
  try { await browser.setWindowRect(0, 0, 1280, 800); } catch (error) { console.log(error.message); }
  await assertAppLease(fixture);
  phase = "routed page with the query";
  await openPageByName(browser, "Press host");
  await browser.waitUntil(() => browser.execute(() => {
    const region = document.querySelector(".main-content .query-results-region");
    return region?.querySelectorAll(".query-group").length === 8
      && region.querySelectorAll(".ls-block:not(.ref-block)").length >= 40;
  }), { timeout: 25_000, timeoutMsg: "The query did not show 40 live results in 8 page groups" });
  const results = [];
  for (const kind of ["between groups"]) {
    phase = kind;
    const aim = await browser.execute(aimAt, kind);
    if (!aim.inResults || aim.onText) throw new Error(`The ${kind} point is not blank space in the results: ${JSON.stringify(aim)}`);
    await browser.action("pointer").move({ x: aim.x, y: aim.y, origin: "viewport" }).down().up().perform();
    const seen = await browser.executeAsync(afterPress);
    results.push({ kind, aim, seen });
    if (seen.editor) throw new Error(`A click on ${kind} opened an editor`);
    if (Math.abs(seen.top - aim.top) > 1) throw new Error(`A click on ${kind} moved the page from ${aim.top} to ${seen.top}`);
    if (seen.rows < 40) throw new Error(`After a click on ${kind} only ${seen.rows} result rows are drawn`);
  }
  fs.writeFileSync(path.join(out, "presses.json"), JSON.stringify(results, null, 2) + "\n");
  if (fs.readFileSync(hostFile, "utf8") !== hostText) throw new Error("The page that holds the query changed on disk");
  console.log("PASS: a click between page groups opens no editor and keeps the page in place");
} catch (error) {
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({
    commit: process.env.TINE_SOURCE_REVISION || "working tree", journey: "GH #1000 query blank press",
    phase, expected: "No editor opens, the scroll position stays within 1 px, the results stay drawn",
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
