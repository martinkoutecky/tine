// GH #915. Intentional OG divergence: indivisible KaTeX remains scrollable.
// Keep ordinary KaTeX breaks and baseline placement; do not rewrite TeX.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import net from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { mathOverflowEquations as equations } from "./lib/math-overflow-fixture.mjs";
import { checkMathTouch } from "./lib/math-touch-pan.mjs";

const root = path.resolve(import.meta.dirname, "..");
await checkMathTouch(root);
// Explicit component-only mode for hosts without a native application binary.
if (process.argv.includes("--touch-only")) process.exit(0);
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/math-touch-pan");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
const source = equations.flatMap(tex => [`- $${tex}$`, `- $$${tex}$$`]).join("\n")
  + "\n- $a + b + c + d + e + f + g + h + i + j + k + l + m + n + o$\n"
  + "- Before $g_2 + \\frac{a}{b}$ after\n";
fs.writeFileSync(path.join(graph, "pages/Math reachability.md"), source);
fs.writeFileSync(path.join(graph, "pages/Math embed.md"), "- {{embed [[Math reachability]]}}\n");
fs.writeFileSync(path.join(graph, "pages/Math query.md"), '- {{query (page "Math reachability")}}\n');
fs.writeFileSync(path.join(graph, "pages/Math preview.md"), "- [[Math reachability]]\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  TINE_E2E_FORCE_MOBILE_DRAWERS: "1",
  TINE_E2E_TOUCH_GESTURES: "android",
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = await freePort(), nativePort = await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn("tauri-driver", webdriverServerArgs(port, nativePort, "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), outcomes: [] };
let browser, stage = "startup";
const shot = async name => {
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(() => done(true))));
  await browser.saveScreenshot(path.join(out, `${name}.png`));
};
async function check(scope, name) {
  stage = name;
  await browser.waitUntil(() => browser.execute(selector => {
    const math = [...(document.querySelector(selector)?.querySelectorAll(".math") ?? [])]
      .filter(element => !element.closest(".linked-references, .unlinked-references, .block-references"));
    return math.length === 8 && math.every(element => element.querySelector(".katex"));
  }, scope), { timeout: 20000, timeoutMsg: `${name}: equations did not typeset` });
  await browser.executeAsync(done => document.fonts.ready.then(() => done(true)));
  const observed = await browser.execute(selector => {
    const container = document.querySelector(selector);
    const surface = container.closest(".peek-popup, .rs-item-body, .main-content");
    const pane = surface.getBoundingClientRect();
    const math = [...container.querySelectorAll(".math")]
      .filter(element => !element.closest(".linked-references, .unlinked-references, .block-references"));
    const tex = element => element.querySelector("annotation")?.textContent ?? "";
    const rows = math.filter(element => tex(element).startsWith("\\mathrm{")).map(element => {
      // Reveal vertically only: scrolling an ancestor sideways would hide the
      // defect by moving the whole page rather than the formula.
      surface.scrollTop += element.getBoundingClientRect().top - surface.getBoundingClientRect().top - 80;
      for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) ancestor.scrollLeft = 0;
      let left = Math.max(0, pane.left), right = Math.min(innerWidth, pane.right);
      for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
        if (getComputedStyle(ancestor).overflowX === "visible") continue;
        const rect = ancestor.getBoundingClientRect();
        left = Math.max(left, rect.left + ancestor.clientLeft);
        right = Math.min(right, rect.left + ancestor.clientLeft + ancestor.clientWidth);
      }
      // An equivalent implementation may wrap or scroll any formula-local box.
      const scrollers = [element, ...element.querySelectorAll("*")].filter(el =>
        el.scrollWidth > el.clientWidth && ["auto", "scroll"].includes(getComputedStyle(el).overflowX));
      scrollers.forEach(el => { el.scrollLeft = el.scrollWidth; });
      const bases = [...element.querySelectorAll(".katex-html > .base")];
      const last = bases.at(-1).getBoundingClientRect();
      const first = bases[0].getBoundingClientRect();
      const endReachable = last.right <= right + 1 && last.right >= left;
      scrollers.forEach(el => { el.scrollLeft = 0; });
      const start = bases[0].getBoundingClientRect();
      return { display: element.classList.contains("math-display"), endReachable,
        startReachable: start.left >= left - 1 && start.left < right,
        scroll: scrollers.map(el => ({ client: el.clientWidth, width: el.scrollWidth })),
        width: first.width, paneWidth: pane.width, visibleWidth: right - left };
    });
    const chain = math.find(element => tex(element).startsWith("a + b"));
    const tops = [...chain.querySelectorAll(".katex-html > .base")].map(el => el.getBoundingClientRect().top);
    // Compare short math against the same KaTeX without the Tine math wrapper.
    const short = math.find(element => tex(element).startsWith("g_2")), row = short.closest(".block-content");
    const reference = row.cloneNode(true);
    reference.querySelector(".math").className = "math-reference";
    reference.style.cssText = `position:absolute;visibility:hidden;width:${row.getBoundingClientRect().width}px`;
    row.after(reference);
    const baseline = content => {
      const marker = document.createElement("span");
      marker.style.cssText = "display:inline-block;width:0;height:0";
      content.prepend(marker);
      const top = content.querySelector(".katex-html .base").getBoundingClientRect().top - marker.getBoundingClientRect().top;
      marker.remove();
      return top;
    };
    const baselineDelta = baseline(row) - baseline(reference);
    reference.remove();
    surface.scrollTop = 0;
    return { rows, baselineDelta, viewportWidth: innerWidth, chainWrapped: Math.max(...tops) > Math.min(...tops) + 1 };
  }, scope);
  proof.outcomes.push({ name, ...observed });
  await shot(`${name}-start`);
  await browser.execute(selector => document.querySelector(selector).querySelectorAll(".math, .math *").forEach(el => {
    if (["auto", "scroll"].includes(getComputedStyle(el).overflowX)) el.scrollLeft = el.scrollWidth;
  }), scope);
  await shot(`${name}-end`);
}
try {
  await new Promise((resolve, reject) => {
    const poll = () => {
      if (driver.exitCode !== null) return reject(new Error("tauri-driver exited"));
      const socket = net.connect(port, "127.0.0.1");
      socket.once("connect", () => { socket.destroy(); resolve(); });
      socket.once("error", () => { socket.destroy(); setTimeout(poll, 50); });
    };
    poll();
  });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await browser.waitUntil(() => browser.execute(() => document.documentElement.dataset.touchGestures === "android"), { timeout: 10000 });
  await assertAppLease(fixture);
  await browser.setWindowSize(640, 1000);
  await openPageByName(browser, "Math reachability");
  await check('[data-pane-id="main"] .page-blocks', "narrow-main");
  await openPageByName(browser, "Math embed");
  await check('[data-pane-id="main"] .embed-block', "embed");
  await openPageByName(browser, "Math query");
  await check('[data-pane-id="main"] .query-block', "query");
  await openPageByName(browser, "Math preview");
  await browser.$('[data-pane-id="main"] .page-ref').moveTo();
  await browser.$(".peek-popup .math").waitForDisplayed({ timeout: 10000 });
  await browser.$(".peek-popup").moveTo();
  await check(".peek-popup", "preview");
  await browser.$('[data-pane-id="main"] h1.page-title').moveTo();
  await browser.waitUntil(async () => !(await browser.$(".peek-popup").isExisting()), { timeout: 10000 });
  await openPageByName(browser, "Math reachability");
  await browser.setWindowSize(1450, 1000);
  for (const [key, name, scope] of [
    ["\uE008", "sidebar", ".right-sidebar .rs-item-body"],
    ["\uE00A", "split", '.pane-leaf:not([data-pane-id="main"]) .page-blocks'],
  ]) {
    await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: key }] }]);
    try { await browser.$('[data-pane-id="main"] h1.page-title').click(); }
    finally { await browser.releaseActions(); }
    await check(scope, name);
  }
  await browser.$('.right-sidebar .rs-close').click();
  await browser.$('[data-tab-strip-pane-id]:not([data-tab-strip-pane-id="main"]) .tab').moveTo();
  await browser.$('[data-tab-strip-pane-id]:not([data-tab-strip-pane-id="main"]) .tab-close').click();
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content").length === 1), { timeout: 10000 });
  // The existing mobile-drawer fixture permits a real 390px native WebView.
  await browser.setWindowSize(390, 1000);
  await browser.waitUntil(() => browser.execute(() => window.innerWidth <= 400), { timeout: 10000 });
  const closeNavigation = await browser.$(".mobile-drawer-close");
  if (await closeNavigation.isDisplayed()) await closeNavigation.click();
  await check('[data-pane-id="main"] .page-blocks', "phone-main");
  assert.ok(proof.outcomes.every(item => item.rows.length === 6 && item.rows.every(row => row.startReachable && row.endReachable)),
    "Every formula's start and end must be reachable inside its pane");
  assert.ok(proof.outcomes.every(item => Math.abs(item.baselineDelta) <= 1), "Short math retains its baseline");
  assert.ok(proof.outcomes.some(item => item.chainWrapped), "KaTeX's ordinary operator breaks remain available");
  assert.equal(fs.readFileSync(path.join(graph, "pages/Math reachability.md"), "utf8"), source);
  fixture.verify();
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL"; proof.stage = stage; proof.observed = String(error.stack);
  proof.expected = "All equation ends reachable at narrow widths; short math baseline and normal breaks preserved";
  proof.classification = error.code === "ERR_ASSERTION" ? "product" : "ambiguous; inspect screenshot and driver log";
  try { await shot("failure"); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`${proof.outcome}: ${out}`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fixture.close(); fs.closeSync(log);
}
