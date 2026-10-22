// GX4 S2: mounted reference answers skip unrelated saves and follow actual
// pane edits/external writes, with unchanged expanded/collapsed behavior.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const out = path.resolve(process.env.REFERENCE_OUT || "artifacts/gx4-s2/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = path.join(run, "graph");
for (const directory of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, directory), { recursive: true });
for (const directory of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, directory));
const pageFile = name => path.join(graph, "pages", `${name}.md`);
fs.writeFileSync(pageFile("Target"), "- Target own body [[Unrelated]]\n");
fs.writeFileSync(pageFile("Linked source"), "- [[Target]] linked seed\n");
fs.writeFileSync(pageFile("Unlinked source"), "- Target plain seed\n");
fs.writeFileSync(pageFile("Tagged source"), "tags:: Target\n\n- tagged seed\n");
fs.writeFileSync(pageFile("Unrelated"), "tags:: Other\n\n- unrelated body\n");
fs.writeFileSync(pageFile("External"), "- unrelated external body\n");
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4886);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, detached: true, stdio: ["ignore", log, log] });
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), diff: "working tree", app, loadAverage: os.loadavg(), passed: [], measurements: [] };
let browser, stage = "startup";
function passed() { report.passed.push(stage); console.log(`PASS: ${stage}`); }
async function shown(selector, text, present = true) {
  await browser.waitUntil(() => browser.execute((selector, text, present) =>
    [...document.querySelectorAll(selector)].some(element => element.textContent.includes(text)) === present, selector, text, present),
  { timeout: 30000, timeoutMsg: `${selector}: expected ${text} ${present ? "present" : "absent"}` });
}
async function referenceBody(kind, source, text) {
  const section = `.pane-leaf[data-pane-id="main"] .${kind}-references`;
  await shown(`${section} .reference-page`, source);
  // Result bodies are virtualized. Bring this source into the viewport before
  // asserting its excerpt; an offscreen placeholder is valid product behavior.
  await browser.execute((section, source) => [...document.querySelectorAll(`${section} .reference-page`)]
    .find(element => element.textContent === source).closest(".reference-group").scrollIntoView({ block: "center" }), section, source);
  await shown(section, text);
}
async function typeIn(scope, token, file, paste = false) {
  await browser.execute((scope, file) => {
    const body = [...document.querySelectorAll(`${scope} .block-content-wrapper`)]
      .find(element => element.textContent.includes(file === "Target" ? "own body" : "unrelated body"));
    if (!body) throw new Error(`missing editable body for ${file}`);
    body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  }, scope, file);
  await browser.$(`${scope} textarea.block-editor`).waitForExist();
  await browser.keys(["Control", "End"]);
  if (paste) {
    // Use the native clipboard and trusted Ctrl+V. A synthetic plain-text
    // paste has no browser default insertion; send-keys exercises bracket
    // auto-pairing instead of inserting this intended literal link.
    const result = await browser.executeAsync((text, done) => {
      window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|write_text", { text })
        .then(() => done({ ok: true }), error => done({ ok: false, error: String(error) }));
    }, token);
    assert(result.ok, result.error || "native clipboard write failed");
    await browser.keys(["Control", "v"]);
  } else await browser.keys(token);
  await waitForFileText(pageFile(file), text => text.includes(token));
  await browser.execute(scope => document.querySelector(`${scope} textarea.block-editor`)?.blur(), scope);
  await delay(2300);
}
async function measure(scope, token, file) {
  await browser.execute(() => { window.__referenceIpc = []; });
  await typeIn(scope, token, file);
  const calls = await browser.execute(() => window.__referenceIpc);
  assert(calls.some(call => call.command === "page_submit"), "must observe a real persisted native save");
  const counts = Object.fromEntries(["get_backlinks", "get_unlinked_refs", "tagged_pages"].map(command =>
    [command, calls.filter(call => call.command === command && call.args.name === "Target").length]));
  report.measurements.push({ scope, counts, loadAverage: os.loadavg() });
  fs.writeFileSync(path.join(run, `${file}-ipc.json`), JSON.stringify(calls, null, 2));
  for (const count of Object.values(counts)) assert.equal(count, 0, "unrelated save refetched Target");
}
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt > 100) throw new Error("WebDriver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60000,
    capabilities: tauriCapabilities(app, "reference-keys") });
  await ensureMainWindow(browser); await browser.setWindowSize(1600, 1000);
  await browser.$(".app-container").waitForExist({ timeout: 60000 });
  await openPageByName(browser, "Target");
  await shown(".main-content .linked-references", "Linked source");
  await browser.execute(() => document.querySelector(".main-content .unlinked-references .references-header").click());
  await shown(".main-content .unlinked-references", "Unlinked source");
  await shown(".main-content .tagged-pages", "Tagged source");
  // The first publication after binding intentionally refreshes everything.
  // Establish that required baseline before measuring precise later saves.
  stage = "establish the first-publication baseline with a visible related edit";
  fs.writeFileSync(pageFile("Linked source"), "- [[Target]] baseline linked\n");
  await shown(".main-content .linked-references", "baseline linked");
  passed();
  await delay(2000);
  await browser.execute(() => {
    const original = window.fetch;
    window.__referenceIpc = [];
    window.fetch = function(input, init) {
      const url = new URL(typeof input === "string" ? input : input.url);
      if ((url.protocol === "ipc:" || url.hostname === "ipc.localhost") && window.__referenceIpc.length < 2000) {
        let args = {};
        if (typeof init?.body === "string") { try { args = JSON.parse(init.body); } catch {} }
        window.__referenceIpc.push({ command: decodeURIComponent(url.pathname.slice(1)), args });
      }
      return original.call(this, input, init);
    };
  });
  stage = "typing on Target does not refetch its reference answers";
  await measure(".main-content .page-blocks", " own-save", "Target"); passed();
  await browser.execute(() => [...document.querySelectorAll(".main-content a.page-ref")].find(element => element.textContent.includes("Unrelated"))
    .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true, button: 0 })));
  await browser.$(".right-sidebar .block-content-wrapper").waitForExist();
  stage = "typing on an unrelated pane does not refetch Target";
  await measure(".right-sidebar", " unrelated-save", "Unrelated"); passed();
  stage = "pane adds a mention and link, then page tags";
  await typeIn(".right-sidebar", " Target [[Target]] pane-linked", "Unrelated", true);
  console.log("Pane mention and link persisted");
  await shown(".main-content .unlinked-references", "pane-linked");
  console.log("Pane mention visible in Target unlinked references");
  await shown(".main-content .linked-references", "pane-linked");
  console.log("Pane link visible in Target linked references");
  // Open the source in a second routed pane for the page-header editor while
  // Target and all three answer sections remain mounted in the first pane.
  await browser.execute(() => [...document.querySelectorAll(".main-content a.page-ref")].find(element => element.textContent.includes("Unrelated"))
    .dispatchEvent(new MouseEvent("click", { bubbles: true, altKey: true, button: 0 })));
  const sourcePane = '.pane-leaf:not([data-pane-id="main"])';
  await browser.$(`${sourcePane} .page-properties .prop-row`).waitForExist();
  await browser.$(`${sourcePane} .page-properties .prop-row`).click();
  const header = await browser.$(`${sourcePane} .page-blocks textarea.block-editor`);
  await header.waitForExist(); await header.click();
  assert((await header.getValue()).includes("tags:: Other"), "edit the actual page tags block");
  await browser.performActions([{ type: "key", id: "replace-page-tags", actions: [
    { type: "keyDown", value: "\uE009" }, { type: "keyDown", value: "a" },
    { type: "keyUp", value: "a" }, { type: "keyUp", value: "\uE009" },
  ] }]);
  await browser.releaseActions();
  await header.addValue("tags:: Target");
  assert.equal(await header.getValue(), "tags:: Target", "native input must preserve both property colons");
  await browser.execute(scope => document.querySelector(`${scope} textarea.block-editor`)?.blur(), sourcePane);
  await waitForFileText(pageFile("Unrelated"), text => text.includes("tags:: Target"));
  await shown(".main-content .tagged-pages", "Unrelated"); passed();
  stage = "external additions and removals update all three sections";
  fs.writeFileSync(pageFile("External"), "tags:: Target\n\n- Target [[Target]] external-related\n");
  await referenceBody("unlinked", "External", "external-related");
  await referenceBody("linked", "External", "external-related");
  await shown(".main-content .tagged-pages", "External");
  fs.writeFileSync(pageFile("External"), "- unrelated external body\n");
  await shown('.pane-leaf[data-pane-id="main"] .unlinked-references .reference-page', "External", false);
  await shown('.pane-leaf[data-pane-id="main"] .linked-references .reference-page', "External", false);
  await shown(".main-content .tagged-pages", "External", false); passed();
  fs.writeFileSync(path.join(run, "visible-final.json"), JSON.stringify(await browser.execute(() => ({
    titles: [...document.querySelectorAll(".page-title")].map(element => element.textContent),
    linked: [...document.querySelectorAll(".main-content .linked-references")].map(element => element.textContent),
    unlinked: [...document.querySelectorAll(".main-content .unlinked-references")].map(element => element.textContent),
    tagged: [...document.querySelectorAll(".main-content .tagged-pages")].map(element => element.textContent),
  })), null, 2));
} catch (error) {
  report.failureCapsule = { testedCommit: report.testedCommit, journey: stage, expected: "Unrelated saves read no Target references; pane and external edits update actual sections", observed: String(error.stack || error), classification: "ambiguous", screenshot: null, log: "failure.html / driver.log" };
  if (browser) fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(run, "report.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ run, ...report }));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
