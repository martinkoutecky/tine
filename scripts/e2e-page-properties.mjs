// Real Tauri/WebKit and disk round-trip for GH #163. The literal reporter
// samples travel through page load -> page actions -> properties panel -> reactive store -> debounced
// native save; helper-only string tests cannot prove this boundary.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4592);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4593);
const TMP = path.join(os.tmpdir(), `tine-page-properties-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const detailed = [
  "alias:: Test Record",
  "ai-prompt:: [[Prompt-Test]]",
  "usage-frequency:: [[Frequency-High]]",
  "",
  "page-level:: [[Level-Two]]",
  "layout:: [[Layout-Top-Collapsed]]",
  "component-state:: [[Component-Wide]]",
  "",
  "timestamp:: 20250707092601",
  "observation-target:: [[Object-Test-Page]]",
  "external-impact::",
  "--:: --",
  "methods:: [[Method-A]] [[Method-B]]",
  "key-conclusion:: [[Conclusion-A]] [[Conclusion-B]]",
  "",
  "- Example content block",
  "",
].join("\n");
const simple = "A:: XX\r\nB:: XX\r\nC:: XX\r\n";
const deletion = "alias:: Delete Me\ncustom/key:: transient\n\n- Body survives\n";
fs.writeFileSync(`${GRAPH}/pages/Property detailed.md`, detailed);
fs.writeFileSync(`${GRAPH}/pages/Property simple.md`, simple);
fs.writeFileSync(`${GRAPH}/pages/Property deletion.md`, deletion);
// GH #210: a table block whose view is set before its title is typed, first on a
// page with no page properties.
const tableConfig = "tine.view:: table\n  tine.table-widths:: prop%3Atest=171";
fs.writeFileSync(`${GRAPH}/pages/Property table.md`, `- ${tableConfig}\n- Other block\n`);
// GH #210 F1: Enter at the start of a page's properties.
const headEnter = "tags:: reading\nicon:: 📚\n\n- Body block\n";
fs.writeFileSync(`${GRAPH}/pages/Property head enter.md`, headEnter);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- [[Property detailed]]\n- [[Property simple]]\n- [[Property deletion]]\n- [[Property table]]\n- [[Property head enter]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  APPDATA: path.join(TMP, "appdata"),
  LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

if (process.platform === "win32" && process.env.CI === "true") {
  spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
}
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(process.env.E2E_ARTIFACT_DIR || TMP, "tauri-driver.log"), "w");
const driverArgs = webdriverServerArgs(
  DRIVER,
  NATIVE,
  process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
);
const driver = spawn(TD, driverArgs, {
  env: webviewTarget.env,
  stdio: ["ignore", log, log],
  detached: process.platform !== "win32",
});
await sleep(2500);
let browser;

async function openPage(name) {
  await browser.$(".nav-all-pages-open").click();
  await browser.$(".all-pages-filter").waitForExist({ timeout: 10_000 });
  await browser.$(".all-pages-filter").setValue(name);
  await browser.waitUntil(async () => (await browser.$$(".all-pages-open")).length > 0, {
    timeout: 5_000, timeoutMsg: "All pages did not reveal fixture pages",
  });
  const opened = await browser.execute((target) => {
    const rows = [...document.querySelectorAll(".all-pages-open")];
    const row = rows.find((element) => element.textContent?.trim() === target);
    if (!row) return { ok: false, rows: rows.map((element) => element.textContent?.trim()) };
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return { ok: true, rows: [] };
  }, name);
  if (!opened.ok) {
    const context = await browser.execute(() => ({
      title: document.querySelector("h1.page-title")?.textContent?.trim(),
      body: document.body.textContent?.trim().slice(0, 1_000),
    }));
    throw new Error(`missing ALL PAGES result ${name}: ${JSON.stringify({ rows: opened.rows, context })}`);
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === name, {
    timeout: 10_000,
    timeoutMsg: `could not open ${name}`,
  });
}

async function openPageProperties() {
  await browser.$("[data-page-actions-trigger]").click();
  const item = await browser.$('[data-page-action-id="page-properties"]');
  await item.waitForExist({ timeout: 5_000 });
  await item.click();
  await browser.$(".page-props-panel").waitForExist({ timeout: 5_000 });
}

async function setPagePropertyField(label, value) {
  await openPageProperties();
  const index = await browser.execute((wanted) => {
    const fields = [...document.querySelectorAll(".page-props-panel .pp-field")];
    return fields.findIndex((field) => field.querySelector(".pp-label")?.textContent?.trim() === wanted);
  }, label);
  if (index < 0) throw new Error(`missing page property field ${label}`);
  const input = (await browser.$$(".page-props-panel .pp-field .pp-input"))[index];
  await input.setValue(value);
  await browser.keys("Enter");
  await browser.$(".page-props-panel").waitForExist({ reverse: true, timeout: 5_000 });
}

async function waitForFile(file, predicate, label) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const text = fs.readFileSync(file, "utf8");
    if (predicate(text)) return text;
    await sleep(100);
  }
  throw new Error(`${label} was not persisted: ${fs.readFileSync(file, "utf8")}`);
}

async function nativeTab({ shift = false } = {}) {
  const actions = shift
    ? [{ type: "keyDown", value: "\uE008" }, { type: "keyDown", value: "\uE004" }, { type: "keyUp", value: "\uE004" }, { type: "keyUp", value: "\uE008" }]
    : [{ type: "keyDown", value: "\uE004" }, { type: "keyUp", value: "\uE004" }];
  await browser.performActions([{ type: "key", id: "page-properties-keyboard", actions }]);
  await browser.releaseActions();
}

async function nativeSelectAll() {
  await browser.performActions([{
    type: "key",
    id: "page-properties-select-all",
    actions: [
      { type: "keyDown", value: "\uE009" },
      { type: "keyDown", value: "a" },
      { type: "keyUp", value: "a" },
      { type: "keyUp", value: "\uE009" },
    ],
  }]);
  await browser.releaseActions();
}

async function nativeArrowDown() {
  // The form traversal above uses a modifier-bearing W3C source. Reset every
  // remote input source before this semantic editor action: a leaked Shift
  // makes the editor correctly leave native selection navigation untouched.
  await browser.releaseActions();
  await browser.performActions([{
    type: "key",
    id: "page-properties-arrow-down",
    actions: [
      { type: "keyDown", value: "\uE015" },
      { type: "keyUp", value: "\uE015" },
    ],
  }]);
  await browser.releaseActions();
}

async function pageArrowDownCapsule(phase) {
  return browser.execute((failurePhase) => {
    const describeEditor = (element) => {
      if (!(element instanceof HTMLTextAreaElement)) return null;
      const block = element.closest("[data-block-id]");
      const surface = element.closest("[data-pane-id], [data-sidebar-surface], [data-surface-id]");
      return {
        value: element.value,
        selection: [element.selectionStart, element.selectionEnd],
        blockId: block?.getAttribute("data-block-id") ?? null,
        surfaceId: surface?.getAttribute("data-pane-id")
          ?? surface?.getAttribute("data-sidebar-surface")
          ?? surface?.getAttribute("data-surface-id")
          ?? null,
      };
    };
    const active = document.activeElement;
    return {
      phase: failurePhase,
      preKey: window.__tinePageArrowDownPreKey ?? null,
      keyWitness: window.__tinePageArrowDownKeyWitness ?? null,
      active: {
        tag: active?.tagName ?? null,
        editor: describeEditor(active),
      },
      editors: [...document.querySelectorAll(".page-blocks textarea")].map(describeEditor),
    };
  }, phase);
}

async function preparePageHeaderArrowDown(expectedValue) {
  return browser.execute((expected) => {
    const header = document.querySelector(".page-blocks textarea.block-editor");
    if (header instanceof HTMLTextAreaElement) {
      header.focus();
      header.setSelectionRange(header.value.length, header.value.length);
    }
    const active = document.activeElement;
    const preKey = {
      isPageHeader: active === header,
      value: active instanceof HTMLTextAreaElement ? active.value : null,
      selection: active instanceof HTMLTextAreaElement ? [active.selectionStart, active.selectionEnd] : null,
      expectedValue: expected,
      expectedSelection: [expected.length, expected.length],
    };
    window.__tinePageArrowDownPreKey = preKey;
    window.__tinePageArrowDownKeyWitness = null;
    document.addEventListener("keydown", (event) => {
      const target = event.target;
      const textarea = target instanceof HTMLTextAreaElement ? target : null;
      const block = textarea?.closest("[data-block-id]");
      const surface = textarea?.closest("[data-pane-id], [data-sidebar-surface], [data-surface-id]");
      const witness = {
        key: event.key,
        flags: {
          shift: event.shiftKey,
          ctrl: event.ctrlKey,
          alt: event.altKey,
          meta: event.metaKey,
          repeat: event.repeat,
          composing: event.isComposing,
          bubbles: event.bubbles,
          cancelable: event.cancelable,
        },
        target: textarea ? {
          value: textarea.value,
          blockId: block?.getAttribute("data-block-id") ?? null,
          surfaceId: surface?.getAttribute("data-pane-id")
            ?? surface?.getAttribute("data-sidebar-surface")
            ?? surface?.getAttribute("data-surface-id")
            ?? null,
        } : null,
      };
      queueMicrotask(() => {
        window.__tinePageArrowDownKeyWitness = { ...witness, defaultPrevented: event.defaultPrevented };
      });
    }, { capture: true, once: true });
    return preKey;
  }, expectedValue);
}

async function replaceHeaderLikeUser(editor, replacement, selection = null) {
  // WebdriverIO's setValue() first clears the textarea as a separate WebDriver
  // command. Clearing an existing header and blurring is a real delete action,
  // so Tine correctly removes that transient editor before setValue() can issue
  // its second (send-keys) command. A user replaces text in one edit session:
  // select it, then type. Exercise that native sequence and prove there was no
  // artificial empty input between the two actions.
  await browser.execute(() => {
    window.__tinePageHeaderInputTrace = [];
    const textarea = document.querySelector(".page-blocks textarea.block-editor");
    textarea?.addEventListener("input", (event) => {
      window.__tinePageHeaderInputTrace.push({
        inputType: event.inputType,
        data: event.data,
        value: event.currentTarget.value,
      });
    }, { capture: true });
  });
  await editor.click();
  if (selection) {
    await browser.execute(({ start, end }) => {
      const textarea = document.querySelector(".page-blocks textarea.block-editor");
      if (!(textarea instanceof HTMLTextAreaElement)) throw new Error("missing page-header editor");
      textarea.focus();
      textarea.setSelectionRange(start, end);
    }, selection);
  } else {
    await nativeSelectAll();
  }
  await editor.addValue(replacement);
  const trace = await browser.execute(() => window.__tinePageHeaderInputTrace ?? []);
  if (
    trace.length === 0
    || trace[0].data !== replacement[0]
    || trace[0].value === ""
    || trace.some((entry) => entry.value === "")
  ) {
    throw new Error(`native page-header replacement emitted an empty intermediate input: ${JSON.stringify(trace)}`);
  }
  return trace;
}

async function deleteHeaderLikeUser(editor) {
  await editor.click();
  await nativeSelectAll();
  await browser.keys(["Backspace"]);
  if (await editor.getValue() !== "") {
    throw new Error(`native page-header deletion did not empty the editor: ${JSON.stringify(await editor.getValue())}`);
  }
  await browser.execute(() => {
    const textarea = document.querySelector(".page-blocks textarea.block-editor");
    if (!(textarea instanceof HTMLTextAreaElement)) throw new Error("missing page-header editor");
    textarea.blur();
  });
  await editor.waitForExist({ reverse: true, timeout: 5_000 });
}

async function activePagePropertyControl() {
  return browser.execute(() => {
    const panel = document.querySelector(".page-props-panel");
    const active = document.activeElement;
    if (!panel || !active || !panel.contains(active)) return null;
    if (active.classList.contains("pp-input")) {
      return active.closest(".pp-field")?.querySelector(".pp-label")?.textContent?.trim() ?? null;
    }
    if (active.matches(".pp-bool input")) return active.closest(".pp-field")?.querySelector(".pp-label")?.textContent?.trim() ?? null;
    return active.classList.contains("pp-done") ? "Done" : null;
  });
}

async function exerciseNativeFormTabTraversal(aliasValue) {
  // Pointer-open the actual routed named-page panel; the key actions below are
  // W3C native actions, not synthetic DOM events, so WebKit performs its focus
  // default action if the capture handler leaves it alone.
  await openPageProperties();
  const aliases = (await browser.$$(".page-props-panel .pp-input"))[0];
  await aliases.click();
  await aliases.setValue(aliasValue);

  await nativeTab();
  if (await activePagePropertyControl() !== "Tags") {
    throw new Error(`native Tab did not leave Aliases for Tags; active=${JSON.stringify(await activePagePropertyControl())}`);
  }
  await waitForFile(
    `${GRAPH}/pages/Property detailed.md`,
    (text) => text.includes(`alias:: ${aliasValue}`),
    "Aliases blur commit after native Tab",
  );

  await nativeTab({ shift: true });
  if (await activePagePropertyControl() !== "Aliases") {
    throw new Error(`native Shift+Tab did not return to Aliases; active=${JSON.stringify(await activePagePropertyControl())}`);
  }

  // Every user-editable property and the add-row participate in native form
  // traversal. Derive the controls from this panel rather than assuming that
  // the five presets are its entire contents (GH #164).
  const tabState = () => browser.execute(() => {
    const panel = document.querySelector(".page-props-panel");
    const controls = [...panel.querySelectorAll("input, button")]
      .filter((control) => !control.disabled && control.tabIndex >= 0);
    return {
      active: controls.indexOf(document.activeElement),
      done: controls.findIndex((control) => control.classList.contains("pp-done")),
      labels: controls.map((control) => control.getAttribute("title")
        || control.closest(".pp-field")?.querySelector(".pp-label")?.textContent?.trim()
        || control.textContent?.trim()),
    };
  });
  const initial = await tabState();
  if (initial.active !== 0 || initial.done !== initial.labels.length - 1) {
    throw new Error(`property form did not expose Aliases through Done: ${JSON.stringify(initial)}`);
  }
  for (let expected = 1; expected <= initial.done; expected++) {
    await nativeTab();
    const state = await tabState();
    if (state.active !== expected) {
      throw new Error(`native Tab did not reach property control ${initial.labels[expected]}: ${JSON.stringify(state)}`);
    }
  }
  // Done owns native focus after traversal; activate it from the keyboard.
  await browser.keys(["Enter"]);
  await browser.$(".page-props-panel").waitForExist({ reverse: true, timeout: 5_000 });
}

try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress),
  });
  await ensureMainWindow(browser);
  // This fixture starts on today's journal so it has a durable navigation
  // source for the seeded pages. Journals render blocks (and a journal title),
  // not a named-page `.page-title`; waiting for the latter prevented openPage()
  // from ever exercising the routed page-properties journey on WebView2.
  await browser.$(".ls-block, .journal-title, .page-title").waitForExist({ timeout: 20_000 });
  // The graph page index warms asynchronously after first paint. Wait for the
  // complete list before using Ctrl+K, otherwise a cold run offers only the
  // misleading "Create page" row for an already-existing file.
  await sleep(3500);

  await openPage("Property detailed");
  await exerciseNativeFormTabTraversal("Test Record, Alternate");
  const customRow = await browser.execute(() => {
    const rows = [...document.querySelectorAll(".page-properties .prop-row")];
    const row = rows.find((element) => element.querySelector(".prop-key")?.textContent?.trim() === "ai-prompt");
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (!customRow) throw new Error("missing rendered custom page property authoring target");
  let headerEditor = await browser.$(".page-blocks textarea.block-editor");
  await headerEditor.waitForExist({ timeout: 5_000 });
  const originalHeader = await headerEditor.getValue();
  if (!originalHeader.includes("ai-prompt:: [[Prompt-Test]]") || !originalHeader.includes("\n\npage-level::")) {
    throw new Error(`page-header ordinary editor lost raw properties/separators: ${JSON.stringify(originalHeader)}`);
  }
  const editedHeader = originalHeader.replace("ai-prompt:: [[Prompt-Test]]", "ai-prompt:: [[Prompt-Edited]]");
  const oldPromptStart = originalHeader.indexOf("Prompt-Test");
  const replacementTrace = await replaceHeaderLikeUser(headerEditor, "Prompt-Edited", {
    start: oldPromptStart,
    end: oldPromptStart + "Prompt-Test".length,
  });
  if ((await headerEditor.getValue()) !== editedHeader) {
    throw new Error(`native page-header replacement did not preserve the intended value: ${JSON.stringify({ replacementTrace, actual: await headerEditor.getValue() })}`);
  }
  const arrowDownPreKey = await preparePageHeaderArrowDown(editedHeader);
  if (
    !arrowDownPreKey.isPageHeader
    || arrowDownPreKey.value !== editedHeader
    || arrowDownPreKey.selection?.[0] !== editedHeader.length
    || arrowDownPreKey.selection?.[1] !== editedHeader.length
  ) {
    throw new Error(JSON.stringify(await pageArrowDownCapsule("pre-key")));
  }
  try {
    await nativeArrowDown();
    await browser.waitUntil(() => browser.execute(() => {
      const active = document.activeElement;
      return active instanceof HTMLTextAreaElement
        && active.closest(".page-blocks") !== null
        && active.classList.contains("block-editor")
        && active.value === "Example content block";
    }), {
      timeout: 5_000,
    });
  } catch {
    throw new Error(JSON.stringify(await pageArrowDownCapsule("post-key")));
  }
  await browser.execute(() => {
    const editor = document.querySelector(".page-blocks textarea.block-editor");
    editor?.focus();
    if (editor instanceof HTMLTextAreaElement) editor.setSelectionRange(2, 2);
  });
  await browser.keys(["ArrowUp"]);
  await browser.waitUntil(async () => (await browser.$(".page-blocks textarea.block-editor").getValue()).includes("Prompt-Edited"), {
    timeout: 5_000,
    timeoutMsg: "Arrow Up did not cross from the first body block back into the page header",
  });
  await browser.execute(() => {
    const textarea = document.querySelector(".page-blocks textarea.block-editor");
    if (!(textarea instanceof HTMLTextAreaElement)) throw new Error("missing page-header editor");
    textarea.blur();
  });
  const detailedAfter = await waitForFile(
    `${GRAPH}/pages/Property detailed.md`,
    (text) => text.includes("alias:: Test Record, Alternate") && text.includes("ai-prompt:: [[Prompt-Edited]]"),
    "detailed page property edit",
  );
  const detailedExpected = detailed
    .replace("alias:: Test Record", "alias:: Test Record, Alternate")
    .replace("ai-prompt:: [[Prompt-Test]]", "ai-prompt:: [[Prompt-Edited]]");
  if (detailedAfter !== detailedExpected) {
    throw new Error(`detailed page changed outside the edited line\nEXPECTED:\n${detailedExpected}\nACTUAL:\n${detailedAfter}`);
  }

  await openPage("Property simple");
  await setPagePropertyField("Icon", "★");
  const simpleAfter = await waitForFile(
    `${GRAPH}/pages/Property simple.md`,
    (text) => text.includes("icon:: ★"),
    "simple page property edit",
  );
  const simpleExpected = "icon:: ★\r\nA:: XX\r\nB:: XX\r\nC:: XX\r\n";
  if (simpleAfter !== simpleExpected) {
    throw new Error(`simple CRLF page changed outside OG's prepended property line\nEXPECTED:\n${JSON.stringify(simpleExpected)}\nACTUAL:\n${JSON.stringify(simpleAfter)}`);
  }
  const lines = simpleAfter.trimEnd().split(/\r?\n/);
  if (lines.some((line) => /^\s*-\s/.test(line) || /^\s{2,}\S/.test(line))) {
    throw new Error(`header properties became outline content: ${JSON.stringify(lines)}`);
  }
  if (!lines.includes("A:: XX") || !lines.includes("B:: XX") || !lines.includes("C:: XX") || !lines.includes("icon:: ★")) {
    throw new Error(`simple page properties were lost or merged: ${JSON.stringify(lines)}`);
  }
  await openPage("Property detailed");
  const reopenedCustom = await browser.execute(() => [...document.querySelectorAll(".page-properties .prop-row")]
    .find((row) => row.querySelector(".prop-key")?.textContent?.trim() === "ai-prompt")
    ?.querySelector(".prop-value")?.textContent?.trim() ?? null);
  if (!reopenedCustom?.includes("Prompt-Edited")) {
    throw new Error(`reopened page did not parse the edited custom header: ${JSON.stringify(reopenedCustom)}`);
  }
  await openPage("Property deletion");
  const deleteTarget = await browser.execute(() => {
    const rows = [...document.querySelectorAll(".page-properties .prop-row")];
    const row = rows.find((element) => element.querySelector(".prop-key")?.textContent?.trim() === "custom/key");
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (!deleteTarget) throw new Error("missing rendered page-header deletion target");
  headerEditor = await browser.$(".page-blocks textarea.block-editor");
  await headerEditor.waitForExist({ timeout: 5_000 });
  const selectAllTrace = await replaceHeaderLikeUser(headerEditor, "alias:: Delete Me Later");
  if ((await headerEditor.getValue()) !== "alias:: Delete Me Later") {
    throw new Error(`Ctrl+A page-header replacement did not preserve the intended value: ${JSON.stringify({ traceLength: selectAllTrace.length, actual: await headerEditor.getValue() })}`);
  }
  await browser.execute(() => {
    const textarea = document.querySelector(".page-blocks textarea.block-editor");
    if (!(textarea instanceof HTMLTextAreaElement)) throw new Error("missing page-header editor");
    textarea.blur();
  });
  const replacementAfter = await waitForFile(
    `${GRAPH}/pages/Property deletion.md`,
    (text) => text.startsWith("alias:: Delete Me Later\n\n"),
    "Ctrl+A page-header replacement",
  );
  if (replacementAfter !== "alias:: Delete Me Later\n\n- Body survives\n") {
    throw new Error(`Ctrl+A page-header replacement changed body bytes: ${JSON.stringify(replacementAfter)}`);
  }
  await openPage("Property deletion");
  const replacedTarget = await browser.execute(() => {
    const row = [...document.querySelectorAll(".page-aliases, .page-properties .prop-row")]
      .find((element) => element.textContent?.includes("Delete Me Later"));
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (!replacedTarget) throw new Error("missing replaced page-header deletion target");
  headerEditor = await browser.$(".page-blocks textarea.block-editor");
  await headerEditor.waitForExist({ timeout: 5_000 });
  await deleteHeaderLikeUser(headerEditor);
  const deletionAfter = await waitForFile(
    `${GRAPH}/pages/Property deletion.md`,
    (text) => !text.includes("alias::") && !text.includes("custom/key::"),
    "deliberate page-header deletion",
  );
  if (deletionAfter !== "- Body survives\n") {
    throw new Error(`deleting the page header changed body bytes: ${JSON.stringify(deletionAfter)}`);
  }
  await openPage("Property deletion");
  if ((await browser.$$(".page-properties .prop-row")).length !== 0) {
    throw new Error("deleted page-header properties reappeared after real-app reopen");
  }
  // GH #210: the table block stays a block through a save, and its title (typed
  // next) neither raises the page-header toast nor stops the page saving.
  await openPage("Property table");
  // The title-less table block is an ordinary block (no page-properties row).
  const openTableBlock = () => browser.execute(() => {
    const content = document.querySelector(".page-blocks .ls-block .block-content");
    if (!content) return false;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if ((await browser.$$(".page-properties .prop-row")).length !== 0) {
    throw new Error("the table block was shown as page properties");
  }
  if (!await openTableBlock()) throw new Error("missing rendered first-bullet table block");
  headerEditor = await browser.$(".page-blocks textarea.block-editor");
  await headerEditor.waitForExist({ timeout: 5_000 });
  const tableHeader = await headerEditor.getValue();
  const widthAt = tableHeader.indexOf("171");
  await replaceHeaderLikeUser(headerEditor, "200", { start: widthAt, end: widthAt + 3 });
  await browser.execute(() => document.querySelector(".page-blocks textarea.block-editor")?.blur());
  const tableSaved = await waitForFile(
    `${GRAPH}/pages/Property table.md`,
    (text) => text.includes("prop%3Atest=200"),
    "table block width edit",
  );
  if (!tableSaved.startsWith("- tine.view:: table") || !tableSaved.includes("- Other block")) {
    throw new Error(`the table block was rewritten as page properties: ${JSON.stringify(tableSaved)}`);
  }
  if (!await openTableBlock()) throw new Error("missing table block for the title edit");
  headerEditor = await browser.$(".page-blocks textarea.block-editor");
  await headerEditor.waitForExist({ timeout: 5_000 });
  // The reporter typed the title on its own line above the config: text at
  // offset 0, then Shift+Enter (a newline inside the block, not a new block).
  // (browser.keys goes to the focused editor without moving its caret, unlike
  // addValue, which jumps a collapsed caret to the end.)
  await headerEditor.click();
  await browser.keys(["Control", "Home"]);
  await browser.keys("test table");
  await browser.keys(["Shift", "Enter"]);
  await browser.execute(() => document.querySelector(".page-blocks textarea.block-editor")?.blur());
  const titled = await waitForFile(
    `${GRAPH}/pages/Property table.md`,
    (text) => text.startsWith("- test table\n  tine.view:: table"),
    "table block title",
  );
  if (!titled.includes("- Other block")) throw new Error(`the table page lost its other block: ${JSON.stringify(titled)}`);
  const tableToasts = await browser.execute(() => [...document.querySelectorAll(".toast-error .toast-msg")].map((node) => node.textContent ?? ""));
  if (tableToasts.length) throw new Error(`typing a table block's title raised an error toast: ${JSON.stringify(tableToasts)}`);
  await browser.$(".page-blocks .sheet-table").waitForExist({
    timeout: 5_000,
    timeoutMsg: "the titled table block no longer renders as a table",
  });
  // GH #210 F1: click the page properties, caret to the start, Enter. OG takes
  // it as a newline in the properties field and its save drops it; before the
  // fix the page stopped saving (the header draft started with a blank line,
  // or a split put an empty block above it, which the store refuses). The
  // page's later edits must still reach the disk.
  await openPageByName(browser, "Property head enter", { entry: process.platform === "win32" ? "button" : "shortcut" });
  const headTarget = await browser.execute(() => {
    const row = [...document.querySelectorAll(".page-properties .prop-row")]
      .find((element) => element.querySelector(".prop-key")?.textContent?.trim() === "tags");
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (!headTarget) throw new Error("missing rendered page-properties row on the head-enter page");
  headerEditor = await browser.$(".page-blocks textarea.block-editor");
  await headerEditor.waitForExist({ timeout: 5_000 });
  await headerEditor.click();
  await browser.keys(["Control", "Home"]);
  await browser.keys(["Enter"]);
  const afterEnter = await browser.execute(() => ({
    editors: [...document.querySelectorAll(".page-blocks textarea.block-editor")].map((t) => t.value),
    blocks: [...document.querySelectorAll(".page-blocks .ls-block")].length,
  }));
  if (afterEnter.editors.length !== 1 || afterEnter.editors[0] !== "\ntags:: reading\nicon:: 📚") {
    throw new Error(`Enter at the start of the page properties did not stay a newline in them: ${JSON.stringify(afterEnter)}`);
  }
  await browser.execute(() => document.querySelector(".page-blocks textarea.block-editor")?.blur());
  // A later edit on the same page: the body block.
  const openBody = await browser.execute(() => {
    const block = [...document.querySelectorAll(".page-blocks .ls-block")]
      .find((element) => element.textContent?.includes("Body block"));
    const content = block?.querySelector(".block-content");
    if (!content) return false;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (!openBody) throw new Error("missing body block on the head-enter page");
  const bodyEditor = await browser.$(".page-blocks textarea.block-editor");
  await bodyEditor.waitForExist({ timeout: 5_000 });
  await browser.keys(["Control", "End"]);
  await browser.keys(" edited");
  await browser.execute(() => document.querySelector(".page-blocks textarea.block-editor")?.blur());
  const headSaved = await waitForFileText(
    `${GRAPH}/pages/Property head enter.md`,
    (text) => text.includes("- Body block edited"),
    { timeoutMs: 10_000 },
  );
  if (headSaved !== "tags:: reading\nicon:: 📚\n\n- Body block edited\n") {
    throw new Error(`Enter at the start of the page properties changed the page: ${JSON.stringify(headSaved)}`);
  }
  const headToasts = await browser.execute(() => [...document.querySelectorAll(".toast-error .toast-msg")].map((node) => node.textContent ?? ""));
  if (headToasts.length) throw new Error(`Enter at the start of the page properties raised an error toast: ${JSON.stringify(headToasts)}`);
  if ((await browser.$$(".page-properties .prop-row")).length !== 2) {
    throw new Error("the page properties no longer render as the page header after Enter at their start");
  }
  console.log(`PASS: page-header click/edit/navigation, native replacements (${replacementTrace.length}+${selectAllTrace.length} input events), deletion, disk bytes, and real-app reopen are canonical`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
    if (process.env.CI === "true") {
      spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
    }
  } else try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(webviewTarget);
  fs.closeSync(log);
}
