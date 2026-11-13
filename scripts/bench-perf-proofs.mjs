// Native PC-3/PC-8/PC-10 probes on the realistic 10k generator. Run through
// dbus-run-session + Xvfb with TINE_APP and PERF_PROOFS_OUT in scratch storage.
// BENCH_EDITOR=cm6 enables the optional engine's existing startup preload.
// Five fresh profiles give five first edits/panels/rebuilds. Budgets use the
// median across launches, like sorted_search_budget, to tolerate isolated load.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { generateRealisticGraph } from "./generate-realistic-graph.mjs";
import { scratchProfile, startTyping, stopTyping, summary, nativeCalibration, editorSelector, editorText } from "./lib/native-editor-bench.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const out = path.resolve(process.env.PERF_PROOFS_OUT || "target/artifacts/perf-proofs-1113/native-bench");
fs.mkdirSync(out, { recursive: true });
const graph = path.join(out, "graph");
const manifest = await generateRealisticGraph({ root: graph, pages: 7000, journals: 3000, seed: 543 });
fs.writeFileSync(path.join(graph, "pages/Perf proof.md"), "- synthetic edit target\n- second activation\n");
const app = path.resolve(process.env.TINE_APP);
const sha256 = crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex");
const commit = execFileSync("git", ["-C", process.cwd(), "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const results = [];
for (let run = 1; run <= 5; run++) {
  const dir = path.join(out, `run-${run}`);
  const env = { ...process.env, ...scratchProfile(dir), TINE_GRAPH: graph,
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const port = await freePort(), nativePort = await freePort();
  const log = fs.openSync(path.join(dir, "driver.log"), "w");
  const driver = spawn("tauri-driver", webdriverServerArgs(port, nativePort, "/usr/bin/WebKitWebDriver"),
    { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  const result = { run, commit, sha256, loadBefore: fs.readFileSync("/proc/loadavg", "utf8").trim() };
  try {
    // Wait for our driver's socket, without starting WebDriver retries that
    // could create an additional app. This is infrastructure readiness only.
    for (let attempt = 0; ; attempt++) {
      try { await fetch(`http://127.0.0.1:${port}/status`); break; }
      catch (error) { if (attempt >= 100) throw error; await new Promise(resolve => setTimeout(resolve, 50)); }
    }
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
      capabilities: tauriCapabilities(app) });
    await ensureMainWindow(browser);
    await browser.setWindowSize(1280, 900);
    await browser.$(".app-container").waitForExist({ timeout: 30000 });
    await openPageByName(browser, "Perf proof");
    // Measure inside the WebView; exclude WebDriver's own round trips. The
    // first activation may correctly use the classic editor during the import.
    result.coldEditMs = await browser.executeAsync(done => {
      const target = document.querySelector(".main-content .block-content-wrapper");
      const at = performance.now();
      target.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      const check = () => {
        const editor = document.querySelector('.main-content textarea.block-editor, .main-content .cm-content[contenteditable="true"]');
        if (editor && document.activeElement === editor) done(performance.now() - at);
        else requestAnimationFrame(check);
      };
      check();
    });
    result.firstEditor = await browser.execute(() => document.querySelector(".main-content textarea.block-editor") ? "textarea" : "cm6");
    const firstEditorText = () => {
      const editor = document.querySelector('.main-content textarea.block-editor, .main-content .cm-content');
      return editor instanceof HTMLTextAreaElement ? editor.value : editor.textContent;
    };
    const firstText = await browser.execute(firstEditorText);
    await browser.keys("x");
    await browser.keys("Backspace");
    assert.equal(await browser.execute(firstEditorText), firstText);
    // During a cold import the fallback stays in place for that activation.
    // A subsequent activation uses the requested engine once its asset lands.
    await browser.waitUntil(async () => {
      if (await browser.$(`.main-content ${editorSelector}`).isExisting()) return true;
      await browser.keys("Escape");
      await browser.execute(() => document.querySelector(".main-content .block-content-wrapper")
        .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
      return false;
    }, { timeout: 10000, timeoutMsg: "the requested editor never became available" });
    await browser.keys("Escape");
    result.calibration = await nativeCalibration(browser);
    result.preferenceReads = await browser.executeAsync(done => {
      (async () => {
        const times = [];
        for (let i = 0; i < 17; i++) {
          const at = performance.now();
          const value = await window.__TAURI_INTERNALS__.invoke("get_app_bool", { key: "show_workspaces", default: true });
          if (value !== true) throw new Error("preference read lost its default");
          if (i) times.push(performance.now() - at);
        }
        return times;
      })().then(done, error => done({ error: String(error) }));
    });
    assert.ok(Array.isArray(result.preferenceReads), JSON.stringify(result.preferenceReads));
    await browser.keys(["t", "s"]);
    await browser.$(".settings-search-input").waitForDisplayed();
    await browser.$(".settings-search-input").setValue("workspaces");
    await browser.$(".settings-search-result").click();
    const toggleSelector = '[data-setting-id="settings-show-workspaces"] [role="switch"]';
    await browser.$(toggleSelector).waitForDisplayed();
    result.preferenceToggles = await browser.executeAsync((selector, done) => {
      (async () => {
        // Tauri's invoke/ipc properties are immutable. Observe its existing
        // custom-protocol fetch and clone the response, preserving callbacks.
        const original = window.fetch;
        const samples = [];
        let writes = 0, reads = 0, finish, bindingGeneration;
        window.fetch = function(input, options, ...rest) {
          const command = new URL(String(input), window.location.href).pathname.split("/").at(-1);
          const args = typeof options?.body === "string" && String(input).startsWith("ipc:") ? JSON.parse(options.body) : null;
          if (args?.bindingGeneration) bindingGeneration = args.bindingGeneration;
          if (args?.key === "show_workspaces" && command === "get_app_bool") reads++;
          const answer = original.call(this, input, options, ...rest);
          if (args?.key === "show_workspaces" && command === "set_app_bool") {
            writes++;
            answer.then(response => {
              if (response.headers.get("Tauri-Response") !== "ok") throw new Error("setting write refused");
              return response.clone().text();
            }).then(() => finish(), error => finish(error));
          }
          return answer;
        };
        try {
          for (let i = 0; i < 16; i++) {
            const toggle = document.querySelector(selector), before = toggle.getAttribute("aria-checked");
            const saved = new Promise((resolve, reject) => { finish = error => error ? reject(error) : resolve(); });
            const at = performance.now();
            toggle.click();
            await saved;
            samples.push(performance.now() - at);
            if (toggle.getAttribute("aria-checked") === before) throw new Error("toggle did not apply");
          }
          return { samples, writes, reads, bindingGeneration };
        } finally { window.fetch = original; }
      })().then(done, error => done({ error: String(error) }));
    }, toggleSelector);
    assert.equal(result.preferenceToggles.writes, 16);
    assert.equal(result.preferenceToggles.reads, 0);
    assert.ok(result.preferenceToggles.bindingGeneration > 0, "the toggle carries its actual graph binding");
    await browser.keys("Escape");
    result.coldPanel = await browser.executeAsync(done => {
      const at = performance.now();
      let previous = at, maxFrameGapMs = 0;
      document.querySelector(".nav-all-pages-open").click();
      const check = () => {
        const now = performance.now();
        maxFrameGapMs = Math.max(maxFrameGapMs, now - previous);
        previous = now;
        const rows = [...document.querySelectorAll(".all-pages-backlinks")];
        if (rows.length && rows.every(row => row.textContent !== "—")) done({ ms: performance.now() - at, maxFrameGapMs });
        else requestAnimationFrame(check);
      };
      check();
    });
    // Arm All Pages first: its shared seed stays cached after navigation, so
    // typing exercises publication patches against the full metric maps.
    await openPageByName(browser, "Perf proof");
    await browser.execute(() => document.querySelector(".main-content .block-content-wrapper")
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
    await browser.$(`.main-content ${editorSelector}`).waitForExist();
    const originalText = await editorText(browser, ".main-content");
    await startTyping(browser);
    const actions = Array.from({ length: 60 }, (_, i) => [
      { type: "keyDown", value: i < 30 ? "x" : "\uE003" }, { type: "keyUp", value: i < 30 ? "x" : "\uE003" }, { type: "pause", duration: 40 },
    ]).flat();
    await browser.performActions([{ type: "key", id: "typing", actions }]);
    await browser.releaseActions();
    const typing = await stopTyping(browser);
    assert.equal(typing.keys.length, 60);
    assert.equal(typing.paint.length, 60);
    assert.equal(typing.focused, true);
    assert.ok(typing.inputs.length >= 30, "all inserted characters reach the editor");
    assert.equal(await editorText(browser, ".main-content"), originalText);
    result.typing = summary(typing.paint);
    result.typingFrameGaps = summary(typing.frames);
    await browser.keys("Escape");
    result.rebuildMs = await browser.executeAsync((bindingGeneration, done) => {
      const at = performance.now();
      window.__TAURI_INTERNALS__.invoke("rescan_graph_now", { rebuild: true, bindingGeneration }).then(
        () => done(performance.now() - at), error => done({ error: String(error) }));
    }, result.preferenceToggles.bindingGeneration);
    assert.equal(typeof result.rebuildMs, "number", JSON.stringify(result.rebuildMs));
    result.loadAfter = fs.readFileSync("/proc/loadavg", "utf8").trim();
    results.push(result);
    console.log(JSON.stringify(result));
  } catch (error) {
    result.failure = String(error.stack);
    result.journey = "native cold edit, preference load/toggle, All Pages and rebuild on realistic 10k";
    result.expected = "focused editor preserves typed input; each toggle writes once; populated panel and rebuild finish";
    result.classification = "ambiguous; inspect failure.png and driver.log";
    await browser?.saveScreenshot(path.join(dir, "failure.png")).catch(() => {});
    throw error;
  } finally {
    fs.writeFileSync(path.join(dir, "result.json"), JSON.stringify(result, null, 2));
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}
const metrics = {
  coldEditMs: summary(results.map(r => r.coldEditMs)),
  preferenceReadMs: summary(results.map(r => summary(r.preferenceReads).median)),
  preferenceToggleMs: summary(results.map(r => summary(r.preferenceToggles.samples).median)),
  coldPanelMs: summary(results.map(r => r.coldPanel.ms)),
  panelMaxFrameGapMs: summary(results.map(r => r.coldPanel.maxFrameGapMs)),
  rebuildMs: summary(results.map(r => r.rebuildMs)),
  typingP95Ms: summary(results.map(r => r.typing.p95)),
  typingMaxFrameGapMs: summary(results.map(r => Math.max(...r.typingFrameGaps.samples))),
};
fs.writeFileSync(path.join(out, "summary.json"), JSON.stringify({ manifest, app, sha256, metrics, results }, null, 2));
console.log(JSON.stringify(metrics, null, 2));
// User-facing limits (PC-8/10) remain fixed. The read/toggle limits include
// native IPC and guarded device storage, independent of graph size.
for (const [key, budget] of Object.entries({ coldEditMs: 100, preferenceReadMs: 20, preferenceToggleMs: 50, panelMaxFrameGapMs: 500, rebuildMs: 10000, typingP95Ms: 16, typingMaxFrameGapMs: 500 })) {
  assert.ok(metrics[key].median < budget, `${key}: ${metrics[key].median} ms exceeds ${budget} ms`);
}
