// Two files share an authored id. Loading A first must never turn B's
// backlink/page-embed row into an editor for A. A safe DTO row is acceptable.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.join(ROOT, "artifacts/og-dup-id-row-resolve");
fs.mkdirSync(OUT, { recursive: true });
const surface = process.env.DUP_SURFACE || "ref";
const label = process.env.DUP_LABEL || "og";
const RUN = fs.mkdtempSync(path.join(OUT, `${label}-${surface}-`));
const GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(RUN, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const U = "58900000-0000-4000-8000-0000000000d3";
const a = `- Original A\n  id:: ${U}\n`;
const b = `- Copied B [[Target]]\n  id:: ${U}\n- B sibling [[Target]]\n`;
const A = path.join(GRAPH, "pages/A.md");
const B = path.join(GRAPH, "pages/B.md");
fs.writeFileSync(A, a);
fs.writeFileSync(B, b);
fs.writeFileSync(path.join(GRAPH, "pages/Target.md"), "- Target body\n");
fs.writeFileSync(path.join(GRAPH, "pages/Host.md"), "- {{embed [[B]]}}\n");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const tested = process.env.DUP_COMMIT || execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const port = Number(process.env.E2E_DRIVER_PORT || 4872);
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"), XDG_CACHE_HOME: path.join(RUN, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver"), ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser, stage = "startup";
const receipt = { tested, app: APP, surface, label };
function shot(name) {
  // WebKitWebDriver's screenshot endpoint can stall independently of the
  // journey. Capture the actual X11 window without blocking its command queue.
  try { execFileSync("import", ["-window", "root", path.join(RUN, name)], { timeout: 10000 }); }
  catch (error) { receipt.screenshotError = String(error); }
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  stage = "load A before B";
  await openPageByName(browser, "A");
  await browser.waitUntil(() => browser.execute(() => document.querySelector('[data-pane-id="main"] .block-content')?.textContent.includes("Original A")), { timeout: 15000 });
  stage = `show B in ${surface}`;
  await openPageByName(browser, surface === "embed" ? "Host" : "Target");
  const row = () => browser.execute((which) => {
    const group = which === "embed" ? document.querySelector('[data-pane-id="main"] .embed-block')
      : [...document.querySelectorAll('[data-pane-id="main"] .reference-group')].find(el => el.querySelector(".reference-page")?.textContent.trim() === "B");
    const content = group?.querySelector(".block-content");
    if (!content) return null;
    return { text: content.textContent, readOnly: content.closest(".ls-block")?.classList.contains("ref-block"),
      // The ordinary sibling proves hydration completed even if the copied
      // row itself must stay on its safe DTO (A -> Target/Host, with B cold).
      hydrated: [...group.querySelectorAll(".ls-block:not(.ref-block) .block-content")].some(el => el.textContent.includes("B sibling")) };
  }, surface);
  await browser.waitUntil(async () => !!(await row())?.hydrated, { timeout: 20000, timeoutMsg: "B's source rows never hydrated" });
  receipt.before = await row();
  stage = "click B's displayed row and type";
  await browser.execute((which) => {
    const group = which === "embed" ? document.querySelector('[data-pane-id="main"] .embed-block')
      : [...document.querySelectorAll('[data-pane-id="main"] .reference-group')].find(el => el.querySelector(".reference-page")?.textContent.trim() === "B");
    const content = group.querySelector(".block-content");
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    content.click();
  }, surface);
  if (!receipt.before.readOnly) {
    const editor = await browser.$("textarea.block-editor");
    await editor.waitForExist({ timeout: 8000 });
    await editor.click();
    await browser.keys(["End", " edited through B", "Escape"]);
    await browser.waitUntil(() => [A, B].some(file => fs.readFileSync(file, "utf8").includes("edited through B")), { timeout: 15000 });
    receipt.edited = true;
  } else {
    assert.equal(await browser.$("textarea.block-editor").isExisting(), false, "read-only B row opened an editor");
    receipt.edited = false;
  }
  stage = "verify source files and visible identity";
  receipt.a = fs.readFileSync(A, "utf8");
  receipt.b = fs.readFileSync(B, "utf8");
  shot("row.png");
  assert.equal(receipt.a, a, "editing B's row changed A's file");
  assert.ok(receipt.before.text.includes("Copied B"), `B's row showed ${JSON.stringify(receipt.before.text)}`);
  if (receipt.edited) assert.ok(receipt.b.includes("edited through B"), "the live edit did not reach B");
  else assert.equal(receipt.b, b, "a read-only row changed B");
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ ...receipt, outcome: "PASS" }, null, 2));
  console.log(`PASS ${label} ${surface}: A unchanged; B ${receipt.edited ? "edited" : "read-only"} (${RUN})`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(RUN, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ ...receipt, outcome: "FAIL", stage, error: String(error) }, null, 2));
  fs.writeFileSync(path.join(RUN, "failure.txt"), `Tested: ${tested}\nJourney: ${surface}: ${stage}\nExpected: B's row shows B; typing changes only B, or the row stays read-only. A is byte-identical.\nObserved: ${error.stack}\nClassification: ${receipt.a && receipt.a !== a ? "product: cross-page write" : "ambiguous; inspect driver log and DOM"}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
