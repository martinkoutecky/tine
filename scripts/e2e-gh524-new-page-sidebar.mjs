// GH #524: distinguish pages with no file from existing empty files.
// Run a custom-protocol build under dbus-run-session + xvfb-run.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const out = path.join(root, "artifacts/gh524");
fs.mkdirSync(out, { recursive: true });
const tmp = fs.mkdtempSync(path.join(out, "native-"));
const graph = path.join(tmp, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(tmp, dir));
const file = name => path.join(graph, "pages", `${name}.md`);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
fs.writeFileSync(file("Source"), "- [[New shift page]]\n- [[New menu page]]\n- [[New main page]]\n- [[New drop page]]\n- Move this subtree\n  - Keep this child\n");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(tmp, "data"), XDG_CONFIG_HOME: path.join(tmp, "config"), XDG_CACHE_HOME: path.join(tmp, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4984);
const log = fs.openSync(path.join(tmp, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  expected: "Fileless sidebar pages expose a first bullet; typing and dropping persist through normal saves", steps: [], failures: [] };
let browser;
async function scenario(name, run) {
  try { await run(); proof.steps.push(name); }
  catch (error) {
    proof.failures.push({ journey: name, observed: String(error), classification: "product-or-harness; inspect DOM and driver log" });
    fs.writeFileSync(path.join(tmp, `failure-${proof.failures.length}.html`), await browser.execute(() => document.body.innerHTML));
    await browser.keys("Escape");
  }
}
async function link(name, menu = false, shift = true) {
  await openPageByName(browser, "Source");
  await browser.execute((name, menu, shift) => {
    const ref = [...document.querySelectorAll(".main-content a.page-ref")].find(el => el.textContent.includes(name));
    if (!ref) throw new Error(`missing link ${name}`);
    ref.dispatchEvent(new MouseEvent(menu ? "contextmenu" : "click", { bubbles: true, cancelable: true, button: menu ? 2 : 0, shiftKey: shift }));
  }, name, menu, shift);
  if (menu) await browser.$('[data-page-action-id="open-sidebar"]').click();
  if (menu || shift) await browser.waitUntil(() => browser.execute(name =>
    document.querySelector(".right-sidebar .rs-item-title")?.textContent.trim() === name, name), { timeout: 5000 });
}
async function edit(scope, name, text, absent = true) {
  if (absent && fs.existsSync(file(name))) throw new Error("opening the page wrote a file before editing");
  const bullet = await browser.$(`${scope} .ls-block .block-content-wrapper`);
  await bullet.waitForExist({ timeout: 5000 });
  await bullet.click();
  await browser.$(`${scope} textarea.block-editor`).waitForExist({ timeout: 5000 });
  await browser.keys(text);
  await browser.keys("Escape");
  await waitForFileText(file(name), body => body.includes(text));
}
const sidebar = ".right-sidebar .rs-item:first-child";
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1200, 900);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await scenario("shift-click a link to a fileless page, edit in sidebar", async () => {
    await link("New shift page");
    await edit(sidebar, "New shift page", "Typed directly in sidebar");
  });
  await scenario("page reference menu opens a fileless page, edit in sidebar", async () => {
    await link("New menu page", true);
    await edit(sidebar, "New menu page", "Typed after page menu");
  });
  await scenario("main pane comparison: plain link to a fileless page", async () => {
    await link("New main page", false, false);
    await edit(".main-content", "New main page", "Typed in main pane");
  });
  await scenario("search Create page, immediately open in sidebar from page menu", async () => {
    await browser.keys(["Control", "k"]);
    await browser.$(".switcher-input").setValue("New search page");
    await browser.waitUntil(() => browser.execute(() => {
      if (document.querySelector('#switcher-results [role="status"]')) return false;
      const row = document.querySelector(".switcher-kind.create")?.closest(".switcher-row");
      if (!row) return false;
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      return true;
    }), { timeout: 10000 });
    await browser.waitUntil(() => browser.execute(() => document.querySelector("h1.page-title")?.textContent.includes("New search page")), { timeout: 10000 });
    await browser.execute(() => document.querySelector("h1.page-title").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 })));
    await browser.$('[data-page-action-id="open-sidebar"]').click();
    await openPageByName(browser, "Source");
    await edit(sidebar, "New search page", "Typed after search creation", false);
  });
  await scenario("drop subtree into a fileless sidebar page", async () => {
    await openPageByName(browser, "Source");
    // Escape from an editor leaves that block selected. Select the actual
    // source here, since a bullet drag intentionally moves the active selection.
    const sourceId = await browser.execute(() => {
      const source = [...document.querySelectorAll(".main-content .block-content-wrapper")].find(el => el.textContent.trim() === "Move this subtree");
      return source.closest("[data-block-id]").getAttribute("data-block-id");
    });
    await browser.$(`.main-content [data-block-id="${sourceId}"] .block-content-wrapper`).click();
    await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 5000 });
    await browser.keys("Escape");
    await link("New drop page");
    await browser.$(`${sidebar} .ls-block`).waitForExist({ timeout: 5000 });
    if (fs.existsSync(file("New drop page"))) throw new Error("drop destination file already exists");
    const points = await browser.execute((scope) => {
      const source = [...document.querySelectorAll(".main-content .ls-block")].find(el => el.querySelector(".block-content-wrapper")?.textContent.trim() === "Move this subtree");
      const a = source.querySelector(".bullet-container").getBoundingClientRect();
      const b = document.querySelector(`${scope} .ls-block .block-main`).getBoundingClientRect();
      return { x: Math.round(a.left + a.width / 2), y: Math.round(a.top + a.height / 2), tx: Math.round(b.left + 25), ty: Math.round(b.top + 4) };
    }, sidebar);
    await browser.performActions([{ type: "pointer", id: "drag", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", x: points.x, y: points.y }, { type: "pointerDown", button: 0 },
      { type: "pointerMove", duration: 400, origin: "viewport", x: points.tx, y: points.ty }, { type: "pointerUp", button: 0 },
    ] }]);
    await browser.releaseActions();
    await waitForFileText(file("New drop page"), body => /^- Move this subtree\r?\n[ \t]+- Keep this child(?:\r?\n|$)/m.test(body));
    await waitForFileText(file("Source"), body => !body.includes("Move this subtree") && !body.includes("Keep this child"));
  });
  if (proof.failures.length) throw new Error(`${proof.failures.length} journeys failed`);
  console.log("PASS", JSON.stringify(proof));
} catch (error) {
  proof.error = String(error);
  console.error("FAIL", JSON.stringify(proof));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(tmp, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  driver.kill("SIGTERM");
  fs.closeSync(log);
}
