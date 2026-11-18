// GH #964 diagnostic journey: literal typing on a routed 548 KB page while
// owner-mediated add/edit commands run. The race itself is pinned in Rust.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

ensurePrivateSessionBus();
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const run = path.resolve(process.env.E2E_TMP_DIR || `artifacts/gh964-${process.pid}`);
assert(!fs.existsSync(run), "use a fresh artifact directory");
const graph = path.join(run, "graph");
for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
const file = path.join(graph, "pages/Large.md");
const seed = Array.from({ length: 10_900 }, (_, i) => `- synthetic block ${String(i).padStart(5, "0")} with ordinary note text here\n`).join("");
assert(seed.length >= 548_000);
fs.writeFileSync(file, seed);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:default-home {:page "Large"}}\n');
const account = installAccountHome(run);
const env = { ...process.env, HOME: path.join(run, "home"), TINE_GRAPH: graph, TINE_RUN: "",
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_STATE_HOME: path.join(run, "state"), XDG_CACHE_HOME: path.join(run, "cache"),
  TAURI_WEBVIEW_AUTOMATION: "true", WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
fs.mkdirSync(env.HOME, { recursive: true });
const port = Number(process.env.E2E_DRIVER_PORT || 4794);
const native = Number(process.env.E2E_NATIVE_PORT || 4795);
const evidence = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  candidate: "uncommitted working tree", bytes: seed.length, lines: 10_900, steps: [], commands: [], keysMs: [] };
let driver, browser;
function cli(args) {
  return new Promise((resolve, reject) => {
    const began = performance.now();
    const child = spawn(app, [...args, "--graph", graph, "--json"], { env, timeout: 60_000 });
    let stdout = "", stderr = "";
    child.stdout.on("data", data => stdout += data);
    child.stderr.on("data", data => stderr += data);
    child.on("error", reject);
    child.on("exit", code => {
      try {
        const result = { args, code, ms: performance.now() - began, reply: JSON.parse(code ? stderr : stdout) };
        evidence.commands.push(result);
        resolve(result);
      } catch (error) { reject(error); }
    });
  });
}
async function type(text) {
  for (const key of text) {
    const began = performance.now();
    await browser.keys([key]);
    evidence.keysMs.push(performance.now() - began);
  }
}
try {
  const log = fs.openSync(path.join(run, "driver.log"), "w");
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
    webdriverServerArgs(port, native, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env, stdio: ["ignore", log, log], detached: true });
  fs.closeSync(log);
  // Driver readiness is transport readiness, independent of page rendering.
  const until = Date.now() + 20_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() >= until) throw new Error("driver unavailable");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 60_000, capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Large");
  await assertAppLease(account);
  await browser.execute(() => {
    window.__gh964Errors = [];
    window.__gh964InputMs = [];
    let started = 0;
    document.addEventListener("input", () => { started = performance.now(); }, true);
    window.addEventListener("input", () => window.__gh964InputMs.push(performance.now() - started));
    window.addEventListener("error", e => window.__gh964Errors.push(e.message));
    window.addEventListener("unhandledrejection", e => window.__gh964Errors.push(String(e.reason)));
  });
  const before = await cli(["page", "show", "Large"]);
  assert.equal(before.code, 0);
  await browser.waitUntil(() => browser.execute(() => {
    const content = document.querySelector(".page .ls-block .block-content-wrapper");
    if (!(content instanceof HTMLElement)) return false;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    content.click();
    return document.activeElement instanceof HTMLTextAreaElement;
  }), { timeout: 15_000 });
  await browser.keys(["End"]);
  await type(" typed-alone");
  await waitForFileText(file, text => text.includes("typed-alone"));
  evidence.steps.push("literal typing without CLI saved on the large routed page");
  // A deterministic sibling refusal, checked in a clean state before the race
  // (the race may leave a retained conflict, which its contract permits, and
  // then nothing typed afterwards reaches disk): composition text is DOM-local
  // and must keep the CLI out until it commits.
  await browser.execute(() => {
    const editor = document.activeElement;
    if (!(editor instanceof HTMLTextAreaElement)) throw new Error("editor lost focus");
    editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    editor.value += " composition-marker";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true, data: " composition-marker" }));
  });
  const held = await cli(["add", "--page", "Large", "- refused-during-composition"]);
  assert.equal(held.code, 6);
  assert.equal(held.reply.error.detail.reason, "unsaved");
  assert(!fs.readFileSync(file, "utf8").includes("refused-during-composition"));
  await browser.execute(() => document.activeElement.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
  await waitForFileText(file, text => text.includes("composition-marker"));
  evidence.steps.push("owner refuses unsaved DOM-local composition; composition later saves");
  const writer = (async () => {
    for (let i = 0; i < 6; i++) {
      const result = await cli(["add", "--page", "Large", `- cli-marker-${i}`]);
      assert([0, 6].includes(result.code), JSON.stringify(result));
    }
    const handle = before.reply.page.blocks[1].handle;
    return cli(["edit", `Large/${handle}`, "cli-edited-other-block"]);
  })();
  await type(" concurrent-typing-marker");
  await writer;
  const visible = await browser.execute(() => ({ text: document.querySelector(".page")?.textContent,
    editors: [...document.querySelectorAll("textarea")].map(el => el.value), errors: window.__gh964Errors,
    inputMs: window.__gh964InputMs }));
  const disk = fs.readFileSync(file, "utf8");
  assert(disk.includes("concurrent-typing-marker") || visible.editors.some(text => text.includes("concurrent-typing-marker")),
    "typed marker must remain on disk or in the live editor");
  for (const result of evidence.commands.filter(item => item.args[0] === "add" && item.code === 0))
    assert(disk.includes(result.args.at(-1)), "every accepted CLI append survives");
  evidence.steps.push("accepted CLI text and concurrent typed text are retained");
  evidence.visible = visible;
  evidence.refusals = evidence.commands.filter(result => result.code !== 0);
  evidence.mountedBlocks = await browser.$$(".page .ls-block").length;
  await browser.saveScreenshot(path.join(run, "result.png"));
  console.log(JSON.stringify({ steps: evidence.steps, refusals: evidence.refusals, bytes: seed.length }));
} catch (error) {
  evidence.failure = { journey: "large routed page typing plus CLI writes", expected: "both sides retained",
    observed: String(error), classification: "ambiguous" };
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  console.error(error);
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(run, "evidence.json"), JSON.stringify(evidence, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
}
