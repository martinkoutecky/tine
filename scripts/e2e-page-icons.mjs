// GH #346: native header typing, sidebar observation and guarded persistence.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "artifacts/gh346", `native-${Date.now()}`);
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, "xdg", dir), { recursive: true });
const fixture = "icon:: 🚩\n\n-\n-\n";
fs.writeFileSync(path.join(graph, "pages/AAA.md"), fixture);
fs.writeFileSync(path.join(graph, "pages/Live.md"), "-\n-\n-\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:favorites ["AAA" "Live"]}\n');
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "xdg/data"), XDG_CONFIG_HOME: path.join(out, "xdg/config"), XDG_CACHE_HOME: path.join(out, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4812);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup exact reporter fixture";
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  delta: "gh346 working diff", iconLookup: process.env.E2E_ICON_BASELINE === "1" ? "unchanged HEAD module" : "working diff", steps: [] };
async function labels(selector) {
  return browser.execute((selector) => {
    const logical = n => n.nodeType === 3 ? n.textContent ?? "" : n.tagName === "IMG" ? n.getAttribute("alt") ?? "" : [...n.childNodes].map(logical).join("");
    return [...document.querySelectorAll(selector)].map(n => logical(n).trim());
  }, selector);
}
async function expectIcon(name, icon, surfaces = ["#sidebar-favorites-list .nav-page-label", "#sidebar-recent-list .nav-page-label"]) {
  for (const selector of surfaces) await browser.waitUntil(async () => (await labels(selector)).includes(`${icon} ${name}`),
    { timeout: 10000, timeoutMsg: `${selector} did not show ${icon} ${name}` });
}
async function menu(name, navigate = true) {
  if (navigate) await openPageByName(browser, name);
  await browser.execute(() => document.querySelector(".page-title").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 150 })));
  await browser.$('[data-page-action-id="page-properties"]').waitForExist();
}
try {
  const deadline = Date.now() + 10000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("driver did not become ready");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/debug/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1400, 1000);
  await browser.$(".page-title").waitForExist({ timeout: 30000 });
  console.log(journey);
  await expectIcon("AAA", "🚩", ["#sidebar-favorites-list .nav-page-label"]);
  await openPageByName(browser, "AAA");
  await expectIcon("AAA", "🚩");
  await menu("AAA");
  await browser.$('[data-page-action-id="open-sidebar"]').click();
  await expectIcon("AAA", "🚩", [".rs-item-title"]);
  // #335 moved the sidebar's All pages list into the full All pages view.
  await browser.$(".nav-all-pages-open").click();
  await expectIcon("AAA", "🚩", [".all-pages-open"]);
  proof.steps.push(journey);

  journey = "type icon:: 🚩 live and see Favorites and Recent before reopening";
  console.log(journey);
  await openPageByName(browser, "Live");
  await browser.$(".page-section .block-content-wrapper").click();
  const editor = await browser.$("textarea.block-editor");
  await editor.waitForExist();
  // WebKitWebDriver drops this supplementary-plane emoji from sendKeys. Send
  // the Unicode editor input event, through the app's ordinary typing handler.
  await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    editor.value = "icon:: 🚩";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "🚩" }));
  });
  await expectIcon("Live", "🚩");
  await browser.keys("Escape");
  const file = path.join(graph, "pages/Live.md");
  await browser.waitUntil(() => fs.readFileSync(file, "utf8").startsWith("icon:: 🚩\n"), { timeout: 10000 });
  proof.steps.push({ journey, persisted: fs.readFileSync(file, "utf8") });

  journey = "external header edit refreshes all four surfaces";
  console.log(journey);
  fs.writeFileSync(path.join(graph, "pages/AAA.md"), fixture.replace("🚩", "AB"));
  await expectIcon("AAA", "AB", ["#sidebar-favorites-list .nav-page-label", "#sidebar-recent-list .nav-page-label", ".rs-item-title"]);
  await browser.$(".nav-all-pages-open").click();
  await expectIcon("AAA", "AB", [".all-pages-open"]);
  await openPageByName(browser, "Live");
  proof.steps.push(journey);

  journey = "menu short text icon saves through the page-property writer";
  console.log(journey);
  await menu("Live", false);
  await browser.$('[data-page-action-id="set-page-icon"]').click();
  await browser.$('[data-page-icon-input]').setValue("XY");
  await browser.keys("Enter");
  await expectIcon("Live", "XY");
  await browser.waitUntil(() => fs.readFileSync(file, "utf8").startsWith("icon:: XY\n"), { timeout: 10000 });
  await menu("Live", false);
  await browser.$('[data-page-action-id="remove-page-icon"]').click();
  await browser.waitUntil(() => !fs.readFileSync(file, "utf8").includes("icon::"), { timeout: 10000 });
  proof.steps.push(journey);
  await browser.saveScreenshot(path.join(out, "page-icons.png"));
} catch (error) {
  proof.failure = { journey, expected: "sidebar icons follow the page property and guarded disk save", observed: String(error), classification: "ambiguous" };
  try { proof.failure.labels = await labels(".nav-page-label, .rs-item-title"); proof.failure.body = await browser?.execute(() => document.body.innerText.slice(0, 2400)); } catch {}
  try { await browser?.saveScreenshot(path.join(out, "failure.png")); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  console.log(`Artifacts: ${out}`);
}
