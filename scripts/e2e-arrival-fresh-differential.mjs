// L8-05: bounded seeded native edits → settled frontend vs a fresh WebView.
// Uses the GX4 S4 driver/navigation setup, without its timing benchmark.
// Build: npm run build; cargo build -p tine --bin tine --features custom-protocol.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "target/artifacts/arrival-small/differential");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(run, dir));
const account = installAccountHome(run);
const ids = ["44444444-4444-4444-8444-000000000001", "44444444-4444-4444-8444-000000000002"];
const pageFile = name => path.join(graph, "pages", `${name}.md`);
const model = ["TODO [[Target]] initial-0", "TODO [[Target]] initial-1"];
for (let i = 0; i < model.length; i++) fs.writeFileSync(pageFile(`Source ${i}`), `- ${model[i]}\n  id:: ${ids[i]}\n`);
fs.writeFileSync(pageFile("Target"), "- target\n");
fs.writeFileSync(pageFile("Other"), "- other\n");
fs.writeFileSync(pageFile("Differential"), `- [[Source 0]]\n- [[Source 1]]\n- ((${ids[0]}))\n- ((${ids[1]}))\n- {{query (and (task TODO) [[Target]])}}\n`);
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug/tine"));
const env = { ...process.env, TINE_GRAPH: graph, HOME: path.join(run, "home"),
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4912);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, detached: true, stdio: ["ignore", log, log] });
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  diff: "working tree", seeds: [0x51a4, 0x109c], stepsPerSeed: 4, passed: [] };
let browser, stage = "launch", operation;

function random(seed) {
  let state = seed;
  return () => { state ^= state << 13; state ^= state >>> 17; state ^= state << 5; return state >>> 0; };
}
async function snapshot() {
  return browser.execute(() => ({
    // Runtime block keys and active selection are deliberately absent. Physical
    // page names, rendered references and result membership must all survive.
    title: document.querySelector("h1.page-title")?.textContent?.trim(),
    queries: document.querySelectorAll(".main-content .query-block").length,
    refs: [...document.querySelectorAll(".main-content .block-ref")].map(node => node.textContent.trim()),
    results: [...document.querySelectorAll(".main-content .query-group")].map(node => node.textContent.trim()).sort(),
    busy: !!document.querySelector('.main-content .query-block[aria-busy="true"], .main-content .query-stale'),
  }));
}
async function settled() {
  const expectedTokens = model.map(text => text.split(" ").at(-1));
  const expectedMatches = model.filter(text => text.startsWith("TODO [[Target]]")).map(text => text.split(" ").at(-1)).sort();
  let answer;
  await browser.waitUntil(async () => {
    answer = await snapshot();
    if (answer.title !== "Differential" || answer.queries !== 1 || answer.busy || answer.refs.length !== 2) return false;
    if (!expectedTokens.every(token => answer.refs.some(text => text.includes(token)))) return false;
    // Token membership is an independent model witness as well as a settlement
    // condition; unchanged or empty stale answers cannot pass by agreeing.
    const actualMatches = answer.results.flatMap(text => [...text.matchAll(/(?:initial-\d|seed-\d+-step-\d+)/g)].map(match => match[0])).sort();
    return JSON.stringify(actualMatches) === JSON.stringify(expectedMatches);
  }, { timeout: 30_000, interval: 100, timeoutMsg: `frontend did not reach model state ${JSON.stringify(model)}` });
  return answer;
}

try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    assert(attempt < 100, "WebDriver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30_000,
    capabilities: tauriCapabilities(app, "arrival-differential") });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1500, 1100);
  await browser.$(".app-container").waitForExist({ timeout: 60_000 });
  await assertAppLease(account);
  await openPageByName(browser, "Differential");
  await settled();
  for (const seed of report.seeds) {
    const next = random(seed);
    for (let step = 0; step < report.stepsPerSeed; step++) {
      const index = next() % 2;
      const marker = next() % 2 ? "TODO" : "DONE";
      const target = next() % 2 ? "Target" : "Other";
      const text = `${marker} [[${target}]] seed-${seed}-step-${step}`;
      operation = { seed, step, index, marker, target };
      stage = "native edit";
      await browser.execute(index => {
        document.querySelector(`.main-content a.page-ref[data-ref="source ${index}"]`)
          .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true, button: 0 }));
      }, index);
      await browser.$(".right-sidebar .block-content-wrapper").waitForExist();
      // Choose the requested page, even when a previous sidebar item remains.
      await browser.execute(id => {
        const block = document.querySelector(`.right-sidebar [data-block-ref="${id}"]`);
        assertBlock(block).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
        function assertBlock(block) {
          if (!block) throw new Error(`source block ${id} is missing`);
          return block.querySelector(".block-content-wrapper");
        }
      }, ids[index]);
      const editor = await browser.$(".right-sidebar textarea.block-editor");
      await editor.waitForExist();
      const copied = await browser.executeAsync((text, done) => window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|write_text", { text })
        .then(() => done(true), () => done(false)), `${text}\nid:: ${ids[index]}`);
      assert(copied, "native clipboard preparation");
      await browser.keys(["Control", "a"]);
      await browser.keys(["Control", "v"]);
      await browser.execute(() => document.querySelector(".right-sidebar textarea.block-editor")?.blur());
      await waitForFileText(pageFile(`Source ${index}`), bytes => bytes.includes(text));
      model[index] = text;
      stage = "settled frontend";
      const live = await settled();
      stage = "fresh frontend";
      await browser.refresh();
      await browser.$(".app-container").waitForExist({ timeout: 60_000 });
      await openPageByName(browser, "Differential");
      const fresh = await settled();
      assert.deepEqual(live, fresh, `seed ${seed} step ${step}: live vs fresh`);
      report.passed.push({ ...operation, live, fresh });
    }
  }
} catch (error) {
  report.failureCapsule = { testedCommit: report.testedCommit, journey: stage, operation,
    expected: "native saved files, settled references/query membership and fresh frontend agree",
    observed: String(error.stack || error), classification: "ambiguous", log: "driver.log" };
  if (browser) {
    await browser.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
  }
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(run, "report.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ run, passed: report.passed.length, failure: report.failureCapsule }));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
