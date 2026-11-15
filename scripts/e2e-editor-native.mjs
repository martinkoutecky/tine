// Shared native editor acceptance. Run E2E_EDITOR=cm6 (default) or textarea.
// Keyboard/pointer and the OS clipboard drive edits; DOM reads only observe.
// The composition probe injects browser events, not an OS IME candidate session.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import crypto from "node:crypto";
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { chooseEditor } from "./lib/e2e-editor-choice.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const mode = process.env.E2E_EDITOR || "cm6";
const propertyValuesOnly = process.env.E2E_PROPERTY_VALUES_ONLY === "1";
assert.ok(["cm6", "textarea"].includes(mode), `Unknown E2E_EDITOR ${mode}`);
assert.ok(["linux", "win32"].includes(process.platform), "Native UI harness supports Linux and Windows only");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/editor-native"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, `${mode}-`));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state", "appdata", "localappdata", "webview"]) fs.mkdirSync(path.join(out, dir));
const file = page => path.join(graph, "pages", `${page}.md`);
const targetId = "12345678-1234-4234-8234-123456789abc";
const sourceBytes = "- block one\n\t- nested child\n- block two\n- keep\n";
// Keep task-cycle bytes deterministic; time-tracking has its own native journey.
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:preferred-workflow :todo :feature/enable-timetracking? false}\n");
for (const [name, text] of Object.entries({
  "Native core": "- alpha\n",
  "Native target": `- zebracorn reference\n  id:: ${targetId}\n`,
  "Native source": sourceBytes,
  "Native copy": "- destination\n",
  "Native cut": "- destination\n",
  "Native compose": "- compose\n",
  "Native property typed": "- Typed property\n",
  "Native property pasted": "- Pasted property\n",
  "Native property existing": "- Existing property\n  alpha:: o\n",
  "Native property values": "- Values\n  alpha:: one\n- More values\n  alpha:: two\n",
  "Native property pending": "- Pending property\n",
})) fs.writeFileSync(file(name), text);
const app = process.env.TINE_APP || path.join(root, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const fixture = accountHomeFixture(out, out, { ...process.env,
  HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  APPDATA: path.join(out, "appdata"), LOCALAPPDATA: path.join(out, "localappdata"),
  E2E_WEBVIEW_USER_DATA_ROOT: path.join(out, "webview"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
});
// The Windows launch helper resolves its profile from the parent process.
process.env.E2E_WEBVIEW_USER_DATA_ROOT = fixture.env.E2E_WEBVIEW_USER_DATA_ROOT;
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = Number(process.env.E2E_DRIVER_PORT) || await freePort();
const nativePort = Number(process.env.E2E_NATIVE_PORT) || await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), platform: process.platform, mode,
  subset: propertyValuesOnly ? "property-values" : "full",
  composition: propertyValuesOnly ? "Not run (property-values subset)"
    : "Synthetic compositionstart/update/end, browser insertText DOM mutation, composing Enter, then ordinary Enter; no real OS IME", outcomes: [] };
let browser, driver, webview, stage = "startup", expected;
const editorSelector = mode === "cm6" ? '.main-content [data-block-editor="cm6"] .cm-content' : ".main-content textarea.block-editor";
const editor = () => browser.$(editorSelector);
async function screenshot(name) {
  // WebKitGTK's WebDriver screenshot can stall with compositing disabled.
  // Capture the actual X11 app window/screen like the other CM6 journeys.
  const target = path.join(out, name);
  if (process.platform === "linux") execFileSync("import", ["-window", "root", target], { env: fixture.env, timeout: 10000 });
  else await browser.saveScreenshot(target);
}
async function keys(...value) { await browser.keys(value); await browser.releaseActions(); }
async function type(text) {
  // Observe each delivered character before sending the next one. Native CM's
  // DOM observer must reconcile the first opener before autopair sees another.
  for (const char of text) {
    const before = await value();
    await browser.keys(char);
    await browser.waitUntil(async () => await value() !== before, { timeout: 5000, timeoutMsg: `Input ${JSON.stringify(char)} was lost` });
  }
}
async function value() {
  return browser.execute(selector => {
    const el = document.querySelector(selector);
    return el instanceof HTMLTextAreaElement ? el.value
      : el ? [...el.querySelectorAll(".cm-line")].map(line => line.textContent).join("\n") : null;
  }, editorSelector);
}
async function saved(page, bytes) {
  expected = { page, bytes };
  assert.equal(await waitForFileText(file(page), text => text === bytes), bytes);
  proof.outcomes.push({ stage, page, bytes });
}
async function edit(page, text) {
  await openPageByName(browser, page, { entry: "button" });
  const block = await browser.$(`//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and contains(., '${text}') and ancestor::*[contains(concat(' ', normalize-space(@class), ' '), ' main-content ')]]`);
  await block.waitForDisplayed();
  await block.click();
  await editor().waitForDisplayed();
}
async function selectedText() {
  return browser.execute(() => {
    const el = document.activeElement;
    return el instanceof HTMLTextAreaElement ? el.value.slice(el.selectionStart, el.selectionEnd) : window.getSelection()?.toString();
  });
}
async function clipboard(command, text) {
  const result = await browser.executeAsync((cmd, text, done) => {
    const timeout = setTimeout(() => done({ error: "Clipboard did not answer" }), 3000);
    globalThis.__TAURI_INTERNALS__.invoke(`plugin:clipboard-manager|${cmd}`, text === null ? {} : { text })
      .then(value => { clearTimeout(timeout); done({ value }); }, error => { clearTimeout(timeout); done({ error: String(error) }); });
  }, command, text ?? null);
  assert.ok(!result.error, result.error);
  return result.value;
}
async function copied(text) {
  await browser.waitUntil(async () => await clipboard("read_text") === text,
    { timeout: 10000, timeoutMsg: `Clipboard never held ${JSON.stringify(text)}` });
}
async function exit() {
  await keys("Escape");
  await browser.waitUntil(async () => !(await editor().isExisting()), { timeout: 5000 });
}
async function reopenSelected() {
  // Escape selected this block. Enter edits it without clicking a rendered
  // reference link or task marker, each of which has its own native action.
  await keys("Enter");
  await editor().waitForDisplayed();
}
async function acceptCompletion(label, exact = true) {
  // The list can retain an older query's rows while the next read is pending.
  // Enter accepts the active row: wait for that semantic target, not merely
  // for the desired label to occur somewhere in a changing list.
  await browser.waitUntil(() => browser.execute((wanted, exact) => {
    const active = document.querySelector(".autocomplete .ac-item.active .ac-label")?.textContent?.trim();
    return exact ? active === wanted : active?.includes(wanted);
  }, label, exact), { timeout: 10000, timeoutMsg: `Completion never selected ${label}` });
  await keys("Enter");
}
async function selectTwoBlocks() {
  await edit("Native source", "block one");
  await exit();
  // The nested child is visible. Extend past it to select the second root too.
  await keys("Shift", "ArrowDown");
  await keys("Shift", "ArrowDown");
  await browser.waitUntil(() => browser.execute(() => {
    const text = [...document.querySelectorAll(".main-content .block-main.selected .block-content")].map(el => el.textContent);
    return text.includes("block one") && text.includes("block two") && !text.includes("keep");
  }), { timeout: 5000, timeoutMsg: "Shift+Down did not select both source blocks" });
}
async function pasteBlocks(page) {
  await edit(page, "destination");
  await keys("End"); await keys("Enter");
  await browser.waitUntil(async () => await value() === "");
  await keys("Control", "v");
  // Cross-page private clipboard payloads resolve their identities asynchronously.
  // Observe the completed transfer before leaving its newly focused last block.
  await waitForFileText(file(page), text => text === "- destination\n- block one\n\t- nested child\n- block two\n");
  await exit();
  await saved(page, "- destination\n- block one\n\t- nested child\n- block two\n");
}
try {
  webview = await startWebdriverApplication(app, fixture.env, nativePort, mode);
  driver = spawn(process.env.TAURI_DRIVER || (process.platform === "win32" ? "msedgedriver" : "tauri-driver"),
    webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env: webview.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
  browser = await remote({
    hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 3, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, mode, process.platform, webview.debuggerAddress),
  });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  stage = "Choose editor through Settings";
  await chooseEditor(browser, mode);

  if (!propertyValuesOnly) {
  stage = "Typing, Enter, Tab, Shift+Tab and Backspace merge";
  await edit("Native core", "alpha");
  await keys("Control", "End"); await type(" beta");
  await keys("Enter"); await type("gamma");
  await keys("Tab"); await saved("Native core", "- alpha beta\n\t- gamma\n");
  await keys("Shift", "Tab"); await saved("Native core", "- alpha beta\n- gamma\n");
  await keys("Home"); await keys("Backspace");
  await saved("Native core", "- alpha betagamma\n");
  await keys("Control", "End"); await keys("Enter"); await type("delta");
  stage = "Up and Down cross block boundaries";
  await keys("Home"); await keys("ArrowUp");
  await browser.waitUntil(async () => await value() === "alpha betagamma");
  await keys("End"); await keys("ArrowDown");
  await browser.waitUntil(async () => await value() === "delta");

  stage = "Shift+arrows, plain text copy/cut/paste and undo/redo";
  await keys("Home"); await keys("Shift", "ArrowRight"); await keys("Shift", "ArrowRight");
  assert.equal(await selectedText(), "de");
  await clipboard("write_text", "sentinel");
  await keys("Control", "c"); await copied("de");
  await keys("End"); await keys("Control", "v");
  await browser.waitUntil(async () => await value() === "deltade");
  await keys("Home"); await keys("Shift", "ArrowRight"); await keys("Shift", "ArrowRight");
  await clipboard("write_text", "cut sentinel");
  await keys("Control", "x"); await copied("de");
  await browser.waitUntil(async () => await value() === "ltade");
  await keys("End"); await keys("Control", "v");
  await saved("Native core", "- alpha betagamma\n- ltadede\n");
  // Tine's document history coalesces consecutive raw edits in one block,
  // including text paste/cut, into one step (document/history.ts).
  await keys("Control", "z"); await saved("Native core", "- alpha betagamma\n-\n");
  await keys("Control", "Shift", "z"); await saved("Native core", "- alpha betagamma\n- ltadede\n");

  stage = "Native pointer double click selects a word";
  const point = await browser.execute(selector => {
    const el = document.querySelector(selector), rect = el.getBoundingClientRect();
    if (el instanceof HTMLTextAreaElement) {
      const style = getComputedStyle(el), canvas = document.createElement("canvas"), ctx = canvas.getContext("2d");
      ctx.font = style.font;
      return { x: Math.round(rect.left + parseFloat(style.paddingLeft) + ctx.measureText("lta").width),
        y: Math.round(rect.top + parseFloat(style.paddingTop) + parseFloat(style.lineHeight) / 2) };
    }
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const text = walker.nextNode(), range = document.createRange();
    range.setStart(text, 2); range.setEnd(text, 3);
    const box = range.getBoundingClientRect();
    return { x: Math.round((box.left + box.right) / 2), y: Math.round((box.top + box.bottom) / 2) };
  }, editorSelector);
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, origin: "viewport", ...point },
    { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
    { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  await browser.waitUntil(async () => await selectedText() === "ltadede", { timeoutMsg: "Double click did not select the word" });
  await keys("End"); await keys("Enter");

  stage = "Page and block autocomplete accept and TODO cycle";
  await type("[[Native tar");
  await acceptCompletion("Native target");
  await saved("Native core", "- alpha betagamma\n- ltadede\n- [[Native target]]\n");
  // Exit trims the optional reference-completion space using the ordinary save.
  await exit();
  await saved("Native core", "- alpha betagamma\n- ltadede\n- [[Native target]]\n");
  await reopenSelected();
  await keys("Control", "End");
  // The default completion adds a space to the live buffer; serialization
  // trims it on disk. Continue with one separator in either live projection.
  await type(`${(await value()).endsWith(" ") ? "" : " "}((zebracorn`);
  await acceptCompletion("zebracorn reference", false);
  // CM6 can project a completed reference as a preview widget. The persisted
  // source, rather than the widget's textContent, is the reference-id oracle.
  await saved("Native core", `- alpha betagamma\n- ltadede\n- [[Native target]] ((${targetId}))\n`);
  for (const marker of ["TODO", "DOING", "DONE", ""]) {
    await keys("Control", "Enter"); await exit();
    await saved("Native core", `- alpha betagamma\n- ltadede\n- ${marker ? `${marker} ` : ""}[[Native target]] ((${targetId}))\n`);
    if (marker) await reopenSelected();
  }
  await saved("Native target", `- zebracorn reference\n  id:: ${targetId}\n`);

  stage = "Multiple block copy/paste preserves source and nested outline";
  await selectTwoBlocks(); await keys("Control", "c");
  await browser.waitUntil(async () => (await clipboard("read_text"))?.includes("block two"));
  await pasteBlocks("Native copy"); await saved("Native source", sourceBytes);
  stage = "Multiple block cut/paste removes exactly the selected subtrees";
  await selectTwoBlocks(); await clipboard("write_text", "block cut sentinel"); await keys("Control", "x");
  await browser.waitUntil(async () => (await clipboard("read_text"))?.includes("block two"));
  await saved("Native source", "- keep\n");
  await pasteBlocks("Native cut"); await saved("Native source", "- keep\n");

  stage = "Synthetic composition commits Unicode; composing Enter does not split";
  await edit("Native compose", "compose"); await keys("Control", "End"); await type(" ");
  const composed = await browser.execute(selector => {
    const el = document.querySelector(selector);
    el.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
    const inserted = document.execCommand("insertText", false, "漢");
    el.dispatchEvent(new CompositionEvent("compositionupdate", { bubbles: true, data: "漢" }));
    el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertCompositionText", data: "漢", isComposing: true }));
    el.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", code: "Enter", keyCode: 229, isComposing: true }));
    return inserted;
  }, editorSelector);
  assert.equal(composed, true, "Browser did not support composition DOM insertion");
  assert.equal(await value(), "compose 漢");
  await browser.execute(selector => document.querySelector(selector).dispatchEvent(
    new CompositionEvent("compositionend", { bubbles: true, data: "漢" })), editorSelector);
  await saved("Native compose", "- compose 漢\n");
  await keys("Enter"); await type("after composition"); await exit();
  await saved("Native compose", "- compose 漢\n- after composition\n");
  }

  // UI-PROPERTY-VALUES-TYPED: use real typing, OS clipboard paste and typing
  // inside an existing value on a routed page, with no property-name choice.
  for (const [method, title] of [["typed", "Typed property"], ["pasted", "Pasted property"], ["existing", "Existing property"]]) {
    stage = `Property values after ${method} entry`;
    const page = `Native property ${method}`;
    await edit(page, title);
    await keys("Control", "End");
    if (method !== "existing") {
      await keys("Shift", "Enter");
      if (method === "typed") await type("alpha:: ");
      else {
        await clipboard("write_text", "alpha:: ");
        await keys("Control", "v");
      }
    } else await type("n");
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")]
      .some(row => row.textContent?.trim() === "one")), { timeout: 10000, timeoutMsg: `${method} property did not offer its known value` });
    await browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-item")]
      .find(row => row.querySelector(".ac-label")?.textContent?.trim() === "one")
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
    await browser.waitUntil(async () => await value() === `${title}\nalpha:: one`, { timeoutMsg: `${method} value choice did not replace the value span` });
    await exit();
    await saved(page, `- ${title}\n  alpha:: one\n`);
  }
  stage = "Pending property values refresh the latest typed query";
  await edit("Native property pending", "Pending property");
  await keys("Control", "End"); await keys("Shift", "Enter");
  // Hold only the native value reply at the read-only fetch boundary. The
  // backend query, editor events, filtering, rendering and save remain real.
  await browser.execute(() => {
    const fetch = window.fetch;
    const pending = window.__pendingProperty = { started: false, release: null };
    window.fetch = function(url, options, ...rest) {
      if (!String(url).endsWith("/query_facets") || JSON.parse(options.body).scope?.key !== "alpha")
        return fetch.call(this, url, options, ...rest);
      pending.started = true;
      return fetch.call(this, url, options, ...rest).then(response => new Promise(resolve => {
        pending.release = () => resolve(response);
      }));
    };
    pending.restore = () => { window.fetch = fetch; };
  });
  await type("alpha:: ");
  await browser.waitUntil(() => browser.execute(() => typeof window.__pendingProperty.release === "function"), { timeout: 10000 });
  await type("tw");
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")]
    .some(row => row.textContent?.trim() === 'Create "tw"')), { timeout: 10000 });
  await browser.execute(() => { window.__pendingProperty.restore(); window.__pendingProperty.release(); });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")]
    .some(row => row.textContent?.trim() === "two") && ![...document.querySelectorAll(".autocomplete .ac-label")]
    .some(row => row.textContent?.trim() === "one")), { timeout: 10000, timeoutMsg: "Pending reply did not refresh the latest query without a keystroke" });
  await screenshot("pending-values.png");
  await browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-item")]
    .find(row => row.querySelector(".ac-label")?.textContent?.trim() === "two")
    .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
  await exit();
  await saved("Native property pending", "- Pending property\n  alpha:: two\n");
  fixture.verify();
  stage = "Capture native evidence";
  await screenshot("complete.png");
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL"; proof.stage = stage; proof.expected = `Complete ${stage}`; proof.expectedFile = expected;
  proof.observed = String(error.stack);
  proof.classification = stage === "Capture native evidence" ? "infrastructure" : "ambiguous; inspect native screenshot, DOM and driver log";
  try { await screenshot("failure.png"); } catch {}
  try { fs.writeFileSync(path.join(out, "failure.html"), await browser.getPageSource()); } catch {}
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  console.log(`${proof.outcome}: ${stage}: ${out}`);
  try { await browser?.deleteSession(); } catch {}
  if (driver?.pid) {
    if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
    else { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
  }
  stopWebdriverApplication(webview);
  fixture.close?.(); fs.closeSync(log);
}
