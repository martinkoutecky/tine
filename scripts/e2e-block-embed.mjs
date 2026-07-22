// Linux real-app proof for GH #124. Exercises the browser's full pointer event
// sequence on embedded disclosure controls and verifies that Enter keeps focus in
// same-page and cross-page transclusions while the source files persist normally.
// A third isolated transclusion covers structural Arrow navigation and deletion.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4478);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4479);
const TMP = "/tmp/tine-block-embed-e2e";
const GRAPH = `${TMP}/graph`;
const SOURCE = `${GRAPH}/pages/Block Embed Source.md`;
const TEST = `${GRAPH}/pages/Block Embed Test.md`;
const CROSS = "11111111-1111-4111-8111-111111111111";
const SAME = "33333333-3333-4333-8333-333333333333";
const NAV = "55555555-5555-4555-8555-555555555555";
const NAV_CHILD = "66666666-6666-4666-8666-666666666666";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(SOURCE, [
  "- Cross-page embedded parent",
  `  id:: ${CROSS}`,
  "  - Cross-page child block",
  "    id:: 22222222-2222-4222-8222-222222222222",
  "    - Cross-page grandchild block",
  "- Navigation-only embedded parent",
  `  id:: ${NAV}`,
  "  - Navigation-only child block",
  `    id:: ${NAV_CHILD}`,
  "    - Navigation-only grandchild block",
  "",
].join("\n"));
fs.writeFileSync(TEST, [
  "- Cross-page block embed",
  `  - {{embed ((${CROSS}))}}`,
  "- Navigation-only block embed",
  `  - {{embed ((${NAV}))}}`,
  "- Same-page source block",
  `  id:: ${SAME}`,
  "  - Same-page child block",
  "    - Same-page grandchild block",
  "- Same-page block embed",
  `  - {{embed ((${SAME}))}}`,
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Block Embed Test]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
const rootSelector = (id) => `.block-embed-host .embed-block [data-block-ref="${id}"]`;

async function openTestPage() {
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  for (const selector of ["a.page-ref=Block Embed Test", "span.page-ref=Block Embed Test", "*=Block Embed Test"]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Block Embed Test", {
    timeout: 10_000, timeoutMsg: "Block Embed Test page did not open",
  });
  await browser.$(`${rootSelector(CROSS)} .block-content`).waitForExist({ timeout: 10_000 });
  await browser.$(`${rootSelector(NAV)} .block-content`).waitForExist({ timeout: 10_000 });
  await browser.$(`${rootSelector(SAME)} .block-content`).waitForExist({ timeout: 10_000 });
}

async function exerciseDisclosure(id, childText) {
  const root = rootSelector(id);
  const toggle = await browser.$(`${root} > .block-main .collapse-toggle.has-children`);
  await toggle.click();
  await browser.waitUntil(() => browser.execute((selector, text) =>
    !(document.querySelector(selector)?.textContent ?? "").includes(text), root, childText), {
    timeout: 5000, timeoutMsg: `${id} did not collapse through a real click`,
  });
  // Solid may replace the control while reconciling the folded children; never
  // reuse the pre-collapse WebDriver element for the expand click. Dispatch the
  // complete WebKit event sequence on the freshly resolved node so this still
  // exercises the macro-host mousedown race that caused the regression.
  const expanded = await browser.execute((selector) => {
    const control = document.querySelector(selector);
    if (!control) return { found: false, survived: false };
    control.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    control.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    const survived = control.isConnected;
    if (survived) control.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
    return { found: true, survived };
  }, `${root} > .block-main .collapse-toggle.has-children`);
  if (!expanded.found || !expanded.survived) throw new Error(`expand control was removed by its pointer sequence: ${JSON.stringify(expanded)}`);
  await browser.waitUntil(() => browser.execute((selector, text) =>
    (document.querySelector(selector)?.textContent ?? "").includes(text), root, childText), {
    timeout: 5000, timeoutMsg: `${id} did not expand through a real click`,
  });
}

async function exerciseEnter(id, inserted, file) {
  const root = rootSelector(id);
  await browser.$(`${root} > .block-main .block-content`).click();
  const editor = await browser.$(`${root} textarea.block-editor`);
  await editor.waitForExist({ timeout: 5000 });
  await browser.execute((selector) => {
    const input = document.querySelector(selector);
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }, `${root} textarea.block-editor`);
  await browser.keys(["Enter"]);
  const focus = await browser.execute((sourceId) => {
    const active = document.activeElement;
    const row = active?.closest?.(".ls-block");
    return {
      editor: active instanceof HTMLTextAreaElement && active.classList.contains("block-editor"),
      inEmbed: Boolean(active?.closest?.(".embed-block")),
      id: row?.getAttribute("data-block-id") ?? null,
      ref: row?.getAttribute("data-block-ref") ?? null,
      sourceId,
    };
  }, id);
  if (!focus.editor || !focus.inEmbed || !focus.id || focus.ref === id) {
    throw new Error(`Enter left the visible embed: ${JSON.stringify(focus)}`);
  }
  await browser.keys([inserted]);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(file, "utf8").includes(inserted), {
    timeout: 10_000, timeoutMsg: `${inserted} was not persisted to ${file}`,
  });
}

async function activeEditor() {
  return browser.execute(() => {
    const active = document.activeElement;
    const row = active?.closest?.(".ls-block");
    return {
      editor: active instanceof HTMLTextAreaElement && active.classList.contains("block-editor"),
      inEmbed: Boolean(active?.closest?.(".embed-block")),
      id: row?.getAttribute("data-block-id") ?? null,
      ref: row?.getAttribute("data-block-ref") ?? null,
      value: active instanceof HTMLTextAreaElement ? active.value : null,
    };
  });
}

async function waitForEmbedEditor(id, value) {
  await browser.waitUntil(async () => {
    const state = await activeEditor();
    return state.editor && state.inEmbed && state.ref === id && (value === undefined || state.value === value);
  }, {
    timeout: 5000,
    timeoutMsg: `embed editor ${id} did not retain focus: ${JSON.stringify(await activeEditor())}`,
  });
}

async function exerciseArrowAndDelete() {
  const childRoot = rootSelector(NAV_CHILD);
  await browser.$(`${childRoot} > .block-main .block-content`).click();
  await browser.execute((selector) => {
    const input = document.querySelector(selector);
    input.focus();
    input.setSelectionRange(0, 0);
  }, `${childRoot} textarea.block-editor`);
  await browser.keys(["ArrowUp"]);
  await waitForEmbedEditor(NAV);

  const parentRoot = rootSelector(NAV);
  await browser.execute((selector) => {
    const input = document.querySelector(selector);
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }, `${parentRoot} textarea.block-editor`);
  await browser.keys(["ArrowDown"]);
  await waitForEmbedEditor(NAV_CHILD);

  await browser.execute((selector) => {
    const input = document.querySelector(selector);
    input.focus();
    input.setSelectionRange(0, input.value.length);
  }, `${childRoot} textarea.block-editor`);
  await browser.keys(["Delete"]);
  await waitForEmbedEditor(NAV_CHILD, "");
  await browser.keys(["Backspace"]);
  await waitForEmbedEditor(NAV);
  await browser.waitUntil(() => !fs.readFileSync(SOURCE, "utf8").includes("Navigation-only child block"), {
    timeout: 10_000,
    timeoutMsg: "empty embedded child was not merged into its source outline",
  });
}

try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await openTestPage();
  await exerciseDisclosure(CROSS, "Cross-page child block");
  await exerciseDisclosure(SAME, "Same-page child block");
  if (fs.readFileSync(SOURCE, "utf8").includes("collapsed::") || fs.readFileSync(TEST, "utf8").includes("collapsed::")) {
    throw new Error("embed-local disclosure leaked collapsed:: into a source file");
  }
  await exerciseArrowAndDelete();
  // WebKitDriver can coalesce immediately repeated identical key events, so keep
  // these proof strings free of doubled characters; the behavior under test is
  // focus routing and source persistence, not keyboard-repeat timing.
  await exerciseEnter(CROSS, "native child alpha", SOURCE);
  await exerciseEnter(SAME, "native child beta", TEST);
  console.log("PASS: embed disclosure, Arrow navigation, deletion, and Enter focus stayed local and persisted safely");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
