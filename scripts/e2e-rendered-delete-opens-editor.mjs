// Delete or Backspace over a read-view text selection opens the
// block's editor with that range selected and writes nothing. On a routed page,
// select `lo w` in rendered `<b>hello world</b>`, press Delete: the file keeps
// its bytes and the editor shows the source a click and shift-click at the two
// ends select (the whole element). Leave, select again, hold Backspace (native
// key down, three repeat keydowns, key up): still no write and the same
// selection. Then Delete in the editor removes what it shows.
// Run once per editor (E2E_EDITOR=textarea or cm6; default cm6):
// source scripts/env.sh; npx tauri build --no-bundle
// E2E_EDITOR=textarea xvfb-run -a node scripts/e2e-rendered-delete-opens-editor.mjs
// E2E_EDITOR=cm6 xvfb-run -a node scripts/e2e-rendered-delete-opens-editor.mjs
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { blurEditor, editorSelector, editorState } from "./lib/e2e-block-editor.mjs";
import { seedClassicEditorProfile } from "./lib/e2e-classic-profile.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const kind = process.env.E2E_EDITOR === "textarea" ? "textarea" : "cm6";
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/rendered-delete-opens-editor"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, `native-${kind}-`));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const PAGE = "Rendered Delete";
const file = path.join(graph, `pages/${PAGE}.md`);
const original = "- <b>hello world</b>\n- after\n";
fs.writeFileSync(file, original);
const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
if (kind === "textarea") seedClassicEditorProfile({ ...fixture.env, E2E_CLASSIC_PROFILE: "1" });
async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = Number(process.env.E2E_DRIVER_PORT || await freePort());
let nativePort = Number(process.env.E2E_NATIVE_PORT || await freePort());
while (nativePort === port) nativePort = await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: execFileSync("git", ["status", "--porcelain"], { cwd: root, encoding: "utf8" }) !== "",
  app, editor: kind, binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Select `lo w` in rendered `<b>hello world</b>` on a routed page; Delete; leave; select again; hold Backspace; Delete in the editor",
  expected: "No byte written by either read-view key; the editor opens on that block with `<b>hello world</b>` selected; the editor's own Delete then removes it", steps: [] };
let browser;
let stage = "startup";
const scope = ".main-content";
const firstRow = () => browser.execute(() => document.querySelector(".main-content .page-section .ls-block")?.dataset.blockId ?? null);

/** Select `lo w` in the first block's rendered text; returns the selected string. */
const selectLoW = () => browser.execute(() => {
  const wrapper = document.querySelector(".main-content .page-section .ls-block > .block-main .block-content-wrapper");
  const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.textContent === "hello world") {
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.setBaseAndExtent(n, 3, n, 7);
      return sel.toString();
    }
  }
  return null;
});

async function expectHandoff(id, what) {
  await browser.$(editorSelector(scope)).waitForExist({ timeout: 10000, timeoutMsg: `${what}: no editor opened` });
  await browser.waitUntil(async () => (await editorState(browser, scope))?.focused, { timeout: 10000, timeoutMsg: `${what}: editor not focused` });
  const state = await editorState(browser, scope);
  if (state.blockId !== id || state.value !== "<b>hello world</b>" || state.start !== 0 || state.end !== 18) {
    throw new Error(`${what}: editor state ${JSON.stringify(state)}`);
  }
  const bytes = fs.readFileSync(file, "utf8");
  if (bytes !== original) throw new Error(`${what}: file changed to ${JSON.stringify(bytes)}`);
}

// Blur, not Escape: Escape leaves a block selection, which Backspace would delete.
async function leaveEditor() {
  await blurEditor(browser, scope);
  await browser.$(editorSelector(scope)).waitForExist({ reverse: true, timeout: 10000 });
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2,
    connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, `rendered-delete-opens-editor-${kind}`) });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  stage = "open the page";
  await openPageByName(browser, PAGE);
  await browser.waitUntil(async () => (await selectLoW()) === "lo w", { timeout: 10000, timeoutMsg: "rendered `hello world` not found" });
  const id = await firstRow();

  stage = "Delete on the rendered selection";
  if ((await selectLoW()) !== "lo w") throw new Error("selection is not `lo w`");
  await browser.keys(["Delete"]);
  await expectHandoff(id, stage);
  proof.steps.push("Delete: no write; the editor opened with `<b>hello world</b>` selected");
  await leaveEditor();
  if (fs.readFileSync(file, "utf8") !== original) throw new Error("leaving the editor wrote the file");

  stage = "held Backspace on the rendered selection";
  if ((await selectLoW()) !== "lo w") throw new Error("selection is not `lo w`");
  await browser.performActions([{ type: "key", id: "kbd", actions: [{ type: "keyDown", value: "" }] }]);
  await browser.$(editorSelector(scope)).waitForExist({ timeout: 10000 });
  // WebDriver gives no native auto-repeat; the platform's repeat keydowns are
  // delivered to the focused editor as these are.
  await browser.execute(() => {
    for (let i = 0; i < 3; i++) document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace", code: "Backspace", repeat: true, bubbles: true, cancelable: true }));
  });
  await browser.performActions([{ type: "key", id: "kbd", actions: [{ type: "keyUp", value: "" }] }]);
  await browser.releaseActions();
  await expectHandoff(id, stage);
  proof.steps.push("held Backspace: no write; the same selection");

  stage = "Delete in the editor";
  await browser.keys(["Delete"]);
  await waitForFileText(file, text => text !== original && !text.includes("hello") && text.includes("- after"));
  proof.steps.push("the editor's own Delete removed the selected source");
  proof.outcome = "PASS";
  console.log(`PASS rendered Delete opens the editor (${kind}): ${out}`);
} catch (error) {
  proof.outcome = "FAIL";
  proof.failure = { stage, observed: String(error.stack), classification: "ambiguous; inspect driver log and DOM",
    file: fs.readFileSync(file, "utf8") };
  if (browser) {
    try { await browser.saveScreenshot(path.join(out, "failure.png")); } catch {}
    try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  }
  console.error(JSON.stringify(proof.failure));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
