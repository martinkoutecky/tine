// ORG-CREATED-FORMAT: on an Org graph, a page created from the Quick Switcher
// is held as Org, so the first property the user adds to it (Copy block ref's
// durable id) lands in the .org file as a :PROPERTIES: drawer, not as a
// Markdown `id::` line that Logseq and Tine never read back as a property.
// Run against a custom-protocol debug build under
// xvfb-run -s "-screen 0 1920x1080x24". Fixtures stay local.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { currentPageTitle } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/org-created-format/native"));
fs.mkdirSync(artifacts, { recursive: true });
const commit = process.env.E2E_TESTED_COMMIT || execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const binarySha256 = createHash("sha256").update(fs.readFileSync(app)).digest("hex");
const port = Number(process.env.E2E_DRIVER_PORT || 4836);
const driverApp = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");

const out = fs.mkdtempSync(path.join(artifacts, "run-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true :preferred-format :org}\n");
fs.writeFileSync(path.join(graph, "pages/Existing.org"), "* An existing page so the index is not empty\n");
const name = "Fresh org page";
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(driverApp, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup";
const proof = { commit, workingTree: process.env.E2E_TESTED_TREE || "org-created-format diff", app, binarySha256, graph, steps: [] };
const check = (value, message) => { if (!value) throw new Error(message); };
const mark = (step) => { journey = step; console.log(step); };
try {
  await browserReady(port);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1200, 900);
  await browser.$(".ls-block, .journal-title, .page-title").waitForExist({ timeout: 30_000 });

  mark("the Quick Switcher's Create row creates the page as an .org file and opens it");
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 10_000 });
  await input.setValue(name);
  // Rows act on mousedown; find and activate the Create row in one round trip.
  await browser.waitUntil(() => browser.execute(() => {
    if (document.querySelector('#switcher-results [role="status"]')) return false;
    const row = document.querySelector(".switcher-row .switcher-kind.create")?.closest(".switcher-row");
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  }), { timeout: 20_000, interval: 150, timeoutMsg: "the Quick Switcher offered no Create row" });
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === name, { timeout: 20_000, timeoutMsg: "the created page did not open" });
  const file = path.join(graph, "pages", `${name}.org`);
  await browser.waitUntil(() => fs.existsSync(file), { timeout: 20_000, timeoutMsg: "the created page has no .org file" });
  proof.steps.push(journey);

  mark("type into the fresh page's block");
  await browser.waitUntil(() => browser.execute(() => {
    const surface = document.querySelector(".main-content .page-blocks .block-content-wrapper");
    if (!(surface instanceof HTMLElement)) return false;
    surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    surface.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    return !!document.querySelector(".main-content textarea.block-editor");
  }), { timeout: 10_000, interval: 200, timeoutMsg: "the fresh page's block did not open an editor" });
  await browser.keys("Fresh target block");
  await browser.keys(["Escape"]);
  await waitForFileText(file, (text) => text.includes("Fresh target block"));
  proof.steps.push(journey);

  mark("Copy block ref writes the block's id as an Org property drawer");
  const opened = await browser.execute(() => {
    const block = [...document.querySelectorAll(".main-content .page-blocks .ls-block")]
      .find((candidate) => candidate.querySelector(".block-content-wrapper")?.textContent?.trim() === "Fresh target block");
    const surface = block?.querySelector(":scope > .block-main > .block-content-wrapper");
    if (!(surface instanceof HTMLElement)) return false;
    const rect = surface.getBoundingClientRect();
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: rect.left + 10, clientY: rect.top + 8, view: window });
    return !surface.dispatchEvent(event) && event.defaultPrevented;
  });
  check(opened, "the fresh block did not open Tine's context menu");
  await browser.$(".ctx-menu").waitForExist({ timeout: 5000 });
  const copyAs = await browser.$(".ctx-menu [role=menuitem][aria-haspopup=menu]");
  await copyAs.moveTo();
  const copyRef = await browser.$('//*[@role="menuitem" and normalize-space(.)="Copy block ref"]');
  await copyRef.waitForDisplayed({ timeout: 5000 });
  await copyRef.click();
  const saved = await waitForFileText(file, (text) => /:id:\s*[0-9a-f-]{36}/i.test(text) || /id::\s*[0-9a-f-]{36}/i.test(text));
  proof.saved = saved;
  check(!/(^|\n)\s*id::/i.test(saved), `the id was written as Markdown into the .org file: ${JSON.stringify(saved)}`);
  check(/:PROPERTIES:\s*\n\s*:id:\s*[0-9a-f-]{36}\s*\n\s*:END:/i.test(saved), `the id is not an Org :PROPERTIES: drawer: ${JSON.stringify(saved)}`);
  proof.steps.push(journey);
  console.log("PASS: a page created from the Quick Switcher on an Org graph takes its first property as Org");
} catch (error) {
  proof.failure = { journey, expected: "a fresh page on an Org graph is edited as Org", observed: String(error), classification: browser ? "ambiguous; inspect DOM and driver log" : "infrastructure: driver/session startup" };
  if (browser) fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  process.exitCode = 1;
  console.error(error);
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  console.log(`Artifacts: ${out}`);
}

async function browserReady(driverPort) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    try { if ((await fetch(`http://127.0.0.1:${driverPort}/status`)).ok) return; } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("tauri-driver did not become ready");
}
