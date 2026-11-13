// H9TvA: native outward copies, synthetic content, no network or graph Git writes.
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as poll } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { saveJourneyScreenshot } from "./lib/e2e-screenshot.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/comment-output-1113/native");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(run, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const source = "- Body passage remains\n  - COMMENTMARKER\n    tine.quote:: Body passage\n    tine.author:: Ada\n    - REPLYMARKER\n- Another body block remains\n";
const pageFile = path.join(graph, "pages/Comment Output.md");
fs.writeFileSync(pageFile, source);
const revision = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const fixture = accountHomeFixture(run, run, { ...process.env, TINE_SOURCE_REVISION: revision,
  TINE_GRAPH: graph, HOME: path.join(run, "home"), XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
async function freePort() {
  const server = net.createServer(); await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
const port = await freePort(), nativePort = await freePort();
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(nativePort)],
  { cwd: root, env: fixture.env, stdio: ["ignore", log, log] });
let browser, stage = "startup";
const proof = { revision, delta: "working tree", run, outcomes: [] };
const shot = name => saveJourneyScreenshot(browser, path.join(run, `${name}.png`));
async function pageAction(text) {
  await browser.$("[data-page-actions-trigger]").click();
  const menu = await browser.$('[role="menu"]'); await menu.waitForExist();
  await menu.$(`.//button[contains(normalize-space(.), '${text}')]`).click();
}
const commentsBox = selector => browser.$(selector).$(".//label[contains(normalize-space(.), 'Include comments')]//input[@type='checkbox']");
try {
  const deadline = Date.now() + 10000;
  for (;;) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("Driver did not become ready");
    await poll(50);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
    connectionRetryTimeout: 30000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/release/tine") } } });
  await ensureMainWindow(browser); await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture); await browser.setWindowSize(1800, 1000);
  await openPageByName(browser, "Comment Output");
  stage = "export includes comments";
  await pageAction("Copy / export"); await browser.$(".export-preview").waitForExist();
  await browser.waitUntil(async () => (await browser.$(".export-preview").getValue()).includes("COMMENTMARKER"));
  const included = await browser.$(".export-preview").getValue();
  assert.match(included, /Comment/); assert.match(included, /REPLYMARKER/);
  assert.equal(await commentsBox(".export-modal").isSelected(), true); await shot("01-export-included");
  stage = "export excludes whole threads";
  await commentsBox(".export-modal").click();
  const omitted = await browser.$(".export-preview").getValue();
  assert.match(omitted, /Body passage remains/); assert.match(omitted, /Another body block remains/);
  assert.doesNotMatch(omitted, /COMMENTMARKER|REPLYMARKER/); await shot("02-export-omitted");
  await browser.$(".export-btn-secondary").click();
  stage = "folder remembers exclusion";
  await pageAction("Share"); await browser.$(".share-dialog").waitForExist();
  await commentsBox(".share-dialog").click();
  await browser.waitUntil(async () => browser.$(".share-export").isEnabled()); await browser.$(".share-export").click();
  await browser.$(".query-export-modal").waitForExist();
  assert.equal(await commentsBox(".query-export-modal").isSelected(), false);
  await browser.waitUntil(async () => browser.$(".query-export-modal .export-btn-primary").isEnabled());
  await shot("03-folder-review"); await browser.$(".query-export-modal .export-btn-primary").click();
  const snapshotFile = path.join(graph, "published-queries/comment-output/app/snapshot.json");
  await waitForFileText(snapshotFile, text => text.includes("Body passage remains"), { timeoutMs: 30000 });
  assert.doesNotMatch(fs.readFileSync(snapshotFile, "utf8"), /COMMENTMARKER|REPLYMARKER/);
  await waitForFileText(path.join(graph, "pages/Exports.md"), text => text.includes("tine.comments:: false"), { timeoutMs: 20000 });
  assert.equal(fs.readFileSync(pageFile, "utf8"), source);
  await browser.$(".query-export-modal").waitForExist({ reverse: true });
  await shot("04-folder-exported");
  proof.outcomes.push("Marked default; omitted root and reply; both body blocks intact", "Folder snapshot excludes thread; definition remembers exclusion; source unchanged");
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2)); console.log(JSON.stringify(proof));
} catch (error) {
  if (browser) await shot("failure").catch(() => {});
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ revision, stage, expected: "Marked comments by default, whole-thread exclusion with body intact, saved folder choice", observed: String(error) }, null, 2)); throw error;
} finally {
  try { if (browser) await browser.deleteSession(); }
  finally {
    driver.kill("SIGTERM");
    await new Promise(resolve => driver.exitCode !== null || driver.signalCode !== null ? resolve() : driver.once("exit", resolve));
    fs.closeSync(log); fixture.close?.(); fixture.verify();
  }
}
