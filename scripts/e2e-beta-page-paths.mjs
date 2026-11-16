// GH #904/#905: disposable profiles, a real checkpoint/session, external moves,
// GUI save, CLI reads/rename and a tine://block/ link. Run under Xvfb.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();

const root = path.resolve(import.meta.dirname, "..");
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/beta-page-paths"));
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state", "runtime"]) fs.mkdirSync(path.join(run, dir), { mode: 0o700 });
const fixture = installAccountHome(run);
const env = { ...process.env, HOME: path.join(run, "home"), TINE_GRAPH: graph, TINE_RUN: "",
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"), XDG_RUNTIME_DIR: path.join(run, "runtime"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11", TAURI_WEBVIEW_AUTOMATION: "true" };
// Full cargo test also produces a dev-mode binary at target/debug/tine.
const app = path.join(run, "tine");
fs.copyFileSync(process.env.TINE_APP || path.join(root, "target/debug/tine"), app);
fs.chmodSync(app, 0o700);
const bench = process.env.TINE_CHECKPOINT_BENCH || path.join(root, "target/debug/examples/checkpoint_launch_bench");
const block = "aaaaaaaa-0000-4000-8000-000000000905";
const original = `- GH905 real content\n  id:: ${block}\n`;
const originalPath = "pages/Survey test.md";
fs.writeFileSync(path.join(graph, originalPath), original);
// The checkpoint benchmark helper expects two linked top-level page names.
fs.writeFileSync(path.join(graph, "pages/Refs.md"), `- [[Survey test]] [[Refs]]\n- ((${block}))\n- [Open block](tine://block/${block})\n`);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:default-home {:page "Refs"} :tine/guide-announced? true}\n');
const receipt = { testedCommit: `${execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim()} + working tree`, passed: [], run };
let browser, driver, log, stage = "cold launch", currentPath = originalPath;
function walk(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? walk(file) : [file];
  });
}
async function until(predicate, message, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await sleep(100);
  }
  throw new Error(message);
}
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
async function launch(label) {
  const port = await freePort(), nativePort = await freePort();
  log = fs.openSync(path.join(run, `${label}.log`), "w");
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
    { env, detached: true, stdio: ["ignore", log, log] });
  await until(() => new Promise(resolve => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    const done = value => { socket.destroy(); resolve(value); };
    socket.once("connect", () => done(true)); socket.once("error", () => done(false));
  }), "WebDriver did not listen");
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Refs");
  await assertAppLease(fixture);
}
async function stop() {
  try { await browser?.deleteSession(); } catch {}
  browser = undefined;
  if (driver) { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
  driver = undefined;
  if (log !== undefined) fs.closeSync(log);
  log = undefined;
}
function screenshot(name) {
  const shot = spawnSync("import", ["-window", "root", path.join(run, name)], { env, timeout: 5_000 });
  if (shot.status !== 0) (receipt.screenshotErrors ??= []).push(String(shot.error || shot.stderr));
}
async function cli(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(app, [...args, "--graph", graph, "--json"], { env, timeout: 60_000 });
    let stdout = "", stderr = "";
    child.stdout.on("data", data => stdout += data); child.stderr.on("data", data => stderr += data);
    child.on("error", reject);
    child.on("exit", code => {
      try { assert.equal(code, 0, `${args.join(" ")}: ${stderr}`); resolve(JSON.parse(stdout)); }
      catch (error) { reject(error); }
    });
  });
}
async function visible(text) {
  await browser.waitUntil(() => browser.execute(text => document.querySelector(".main-content .page-blocks")?.textContent.includes(text), text),
    { timeout: 20_000, timeoutMsg: `real page content did not show: ${text}` });
}
async function checkCli(name) {
  const listed = await cli(["pages"]), shown = await cli(["page", "show", name]);
  assert.equal(listed.pages.find(page => page.name === name)?.id, currentPath);
  assert.equal(shown.id, currentPath);
  const blockShown = await cli(["block", "show", block]);
  assert.equal(blockShown.block.group.page, name);
  fs.writeFileSync(path.join(run, `cli-${receipt.passed.length}.json`), JSON.stringify({ listed, shown, blockShown }, null, 2));
}
async function deepLink(name = "Survey test") {
  await openPageByName(browser, "Refs");
  await browser.$(`.main-content a[href="tine://block/${block}"]`).click();
  // The referrer itself displays the referenced text. Observe the destination
  // route and its actual block, so that text cannot satisfy the link check.
  await browser.waitUntil(() => browser.execute((name, block) =>
    document.querySelector(".main-content .zoom-breadcrumb .crumb-page")?.textContent.trim() === name
      && Boolean(document.querySelector(`.main-content .zoomed-block .ls-block[data-block-ref="${block}"]`)), name, block),
  { timeout: 20_000, timeoutMsg: `tine://block did not route to ${name}` });
  await visible("GH905");
  assert(!await browser.execute(() => [...document.querySelectorAll(".toast-msg")].some(node => node.textContent.includes("not found"))));
}
try {
  await launch("cold");
  await openPageByName(browser, "Survey test");
  await visible("GH905 real content");
  await openPageByName(browser, "Refs");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content .block-ref")?.textContent.includes("GH905")), { timeout: 20_000 });
  await browser.execute(() => document.querySelector(".main-content .block-ref").dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".right-sidebar")?.textContent.includes("GH905 real content")), { timeout: 20_000 });
  await until(() => walk(path.join(run, "data")).some(file => file.includes("sessions/") && file.endsWith(".json") && fs.readFileSync(file, "utf8").includes(originalPath)), "session did not persist the sidebar's old path");
  await until(() => walk(path.join(run, "data")).some(file => file.includes("launch-checkpoints/") && file.endsWith(".bin")), "cold launch wrote no checkpoint");
  const checkpoint = walk(path.join(run, "data")).find(file => file.includes("launch-checkpoints/") && file.endsWith(".bin"));
  receipt.passed.push("cold page opened; checkpoint and sidebar session record its original path");
  screenshot("cold-sidebar.png");
  await stop();

  stage = "move while closed, then warm restart with stale sidebar";
  currentPath = "pages/shared-elias/Survey test.md";
  fs.mkdirSync(path.dirname(path.join(graph, currentPath)), { recursive: true });
  fs.renameSync(path.join(graph, originalPath), path.join(graph, currentPath));
  await launch("warm");
  await openPageByName(browser, "Survey test");
  await visible("GH905 real content");
  assert(fs.readFileSync(path.join(run, "warm.log"), "utf8").includes("tine launch checkpoint served"));
  await checkCli("Survey test");
  await deepLink();
  assert.equal(fs.readFileSync(path.join(graph, currentPath), "utf8"), original);
  receipt.passed.push(stage);

  stage = "GUI edit saves only the real moved file";
  await openPageByName(browser, "Survey test");
  await visible("GH905 real content");
  await browser.waitUntil(async () => {
    if (await browser.$(".main-content textarea.block-editor").isExisting()) return true;
    await browser.$(".main-content .page-blocks .block-content [data-so]").click();
    return browser.$(".main-content textarea.block-editor").isExisting();
  }, { timeout: 10_000, timeoutMsg: "clicking the real page text did not enter editing" });
  const editor = await browser.$(".main-content textarea");
  await editor.waitForExist({ timeout: 10_000 });
  const raw = await editor.getValue();
  await browser.keys(["Control", "a"]);
  await browser.keys(raw.replace("GH905 real content", "GH905 saved real content"));
  await browser.keys(["Escape"]);
  await until(() => fs.readFileSync(path.join(graph, currentPath), "utf8").includes("GH905 saved real content"), "GUI edit did not save to the moved file");
  assert(!fs.existsSync(path.join(graph, originalPath)));
  receipt.passed.push(stage);
  screenshot("warm-saved.png");

  stage = "sync-style copy then removal while running";
  await openPageByName(browser, "Refs");
  const previous = currentPath;
  currentPath = "pages/live/Survey test.md";
  fs.mkdirSync(path.dirname(path.join(graph, currentPath)), { recursive: true });
  fs.copyFileSync(path.join(graph, previous), path.join(graph, currentPath));
  fs.unlinkSync(path.join(graph, previous));
  await until(async () => {
    receipt.lastLiveListing = await cli(["pages"]);
    return receipt.lastLiveListing.pages.find(page => page.name === "Survey test")?.id === currentPath;
  }, "live move did not replace the listed path");
  await openPageByName(browser, "Survey test");
  await visible("GH905 saved real content");
  await checkCli("Survey test");
  await deepLink();
  receipt.passed.push(stage);

  stage = "CLI rename retains the current folder";
  await cli(["page", "rename", "Survey test", "Survey renamed"]);
  currentPath = "pages/live/Survey renamed.md";
  assert(fs.existsSync(path.join(graph, currentPath)));
  assert(!fs.existsSync(path.join(graph, "pages/Survey renamed.md")));
  await checkCli("Survey renamed");
  receipt.passed.push(stage);
  await stop();

  stage = "old flattened rename result moved back by hand, then restart";
  // Recreate old #904's on-disk result, checkpoint it through the real store,
  // and undo that flattening while closed. No private or reporter graph.
  const flattened = "pages/Survey renamed.md";
  fs.renameSync(path.join(graph, currentPath), path.join(graph, flattened));
  const written = spawnSync(bench, ["write", graph, checkpoint], { env, encoding: "utf8", timeout: 60_000 });
  assert.equal(written.status, 0, String(written.error || written.stderr));
  assert(written.stdout.includes("Written"), written.stdout);
  fs.writeFileSync(path.join(run, "flattened-checkpoint.json"), written.stdout);
  fs.renameSync(path.join(graph, flattened), path.join(graph, currentPath));
  await launch("manual-back");
  await openPageByName(browser, "Survey renamed");
  await visible("GH905 saved real content");
  await checkCli("Survey renamed");
  await deepLink("Survey renamed");
  assert(fs.readFileSync(path.join(run, "manual-back.log"), "utf8").includes("tine launch checkpoint served"));
  assert(!fs.existsSync(path.join(graph, flattened)));
  receipt.passed.push(stage);
  screenshot("manual-back.png");
  receipt.outcome = "passed";
} catch (error) {
  receipt.outcome = "failed";
  receipt.capsule = { stage, expected: "real moved page opens and saves at its current path, and CLI/block links agree after restart", observed: String(error), classification: "ambiguous" };
  screenshot("failure.png");
  if (browser) fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  process.exitCode = 1;
} finally {
  await stop();
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify(receipt, null, 2) + "\n");
  console.log(JSON.stringify(receipt));
}
