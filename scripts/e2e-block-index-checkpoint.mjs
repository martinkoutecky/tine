// Native observation of exact block ownership after watcher delivery and a
// checkpoint restart. Build the app with custom-protocol and the existing
// checkpoint_launch_bench example with test-faults before running under Xvfb.
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-block-index-dump/native"));
fs.mkdirSync(OUT, { recursive: true });
const run = fs.mkdtempSync(path.join(OUT, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const id = "aaaaaaaa-0000-4000-8000-000000000001";
const missing = "aaaaaaaa-0000-4000-8000-0000000000ff";
const refs = `- See ((${id}))\n- {{embed ((${id}))}}\n- Dangling ((${missing}))\n- [[Source]] [[Destination]]\n`;
fs.writeFileSync(path.join(graph, "pages/Refs.md"), refs);
fs.writeFileSync(path.join(graph, "pages/Source.md"), `- before move\n  id:: ${id}\n`);
fs.writeFileSync(path.join(graph, "pages/Destination.md"), "- destination keeper\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const app = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const bench = process.env.TINE_CHECKPOINT_BENCH || path.join(ROOT, "target/release/examples/checkpoint_launch_bench");
const driver = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
let browser, processGroup, log;
let stage = "launch";
const testedCommit = process.env.TINE_TESTED_COMMIT
  || `${spawnSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).stdout?.trim() || "unknown"} + working tree`;
const receipt = { testedCommit, passed: [] };

async function shot(name) {
  // Like bench-startup.mjs, capture the displayed window: WebKit's screenshot
  // endpoint can stall while the semantic WebDriver checks have already passed.
  const result = spawnSync("import", ["-window", "root", path.join(run, name)], { env, timeout: 5_000 });
  if (result.status !== 0) (receipt.screenshotErrors ??= []).push(String(result.error || result.stderr));
}

async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
async function waitForPort(port) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const ready = await new Promise(resolve => {
      const socket = net.createConnection({ host: "127.0.0.1", port });
      const finish = ready => { socket.destroy(); resolve(ready); };
      socket.once("connect", () => finish(true));
      socket.once("error", () => finish(false));
      socket.setTimeout(500, () => finish(false));
    });
    if (ready) return;
    if (processGroup.exitCode !== null) throw new Error("the WebDriver process exited before listening");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`WebDriver port ${port} did not listen`);
}
async function launch(label) {
  const port = await freePort(), native = await freePort();
  log = fs.openSync(path.join(run, `${label}.log`), "w");
  processGroup = spawn("dbus-run-session", ["--", driver, "--port", String(port), "--native-port", String(native), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
    { env, stdio: ["ignore", log, log], detached: true });
  await Promise.all([waitForPort(port), waitForPort(native)]);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 20_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Refs");
}
async function stop() {
  try { await browser?.deleteSession(); } catch {}
  browser = undefined;
  if (processGroup) { try { process.kill(-processGroup.pid, "SIGKILL"); } catch {} }
  processGroup = undefined;
  if (log !== undefined) fs.closeSync(log);
  log = undefined;
}
async function visible(text) {
  await browser.waitUntil(() => browser.execute((wanted) => {
    const reference = [...document.querySelectorAll(".page-blocks .block-ref")].some(node => node.textContent.includes(wanted));
    const embed = [...document.querySelectorAll(".page-blocks .embed-block")].some(node => node.textContent.includes(wanted));
    return reference && embed;
  }, text), { timeout: 20_000, timeoutMsg: `reference and embed did not show ${text}` });
}
function checkpoints(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const child = path.join(dir, entry.name);
    return entry.isDirectory() ? checkpoints(child) : child.includes("launch-checkpoints") && child.endsWith(".bin") ? [child] : [];
  });
}
try {
  await launch("cold");
  await visible("before move");
  stage = "move delivered by the watcher";
  fs.writeFileSync(path.join(graph, "pages/Destination.md"), `- after move\n  id:: ${id}\n`);
  fs.writeFileSync(path.join(graph, "pages/Source.md"), "- source keeper\n");
  await openPageByName(browser, "Source");
  await openPageByName(browser, "Refs");
  await visible("after move");
  assert.equal(fs.readFileSync(path.join(graph, "pages/Refs.md"), "utf8"), refs);
  receipt.passed.push(stage);
  await shot("moved.png");

  stage = "checkpoint restart";
  // Obtain the real app-data destination, then write the final generation
  // through the store's existing checkpoint writer after stopping the app.
  await browser.waitUntil(() => checkpoints(path.join(run, "data")).length === 1,
    { timeout: 30_000, timeoutMsg: "the cold launch did not write its first idle checkpoint" });
  const [checkpoint] = checkpoints(path.join(run, "data"));
  await stop();
  const written = spawnSync("prlimit", ["--as=2147483648", bench, "write", graph, checkpoint], { env, encoding: "utf8" });
  assert.equal(written.status, 0, written.stderr);
  assert(JSON.parse(written.stdout.trim().split("\n").at(-1)).write?.includes("Written"), "the checkpoint writer skipped the final generation");
  fs.writeFileSync(path.join(run, "checkpoint-write.json"), written.stdout);
  await launch("warm");
  await visible("after move");
  assert(fs.readFileSync(path.join(run, "warm.log"), "utf8").includes("tine launch checkpoint served"), "the restart did not serve the checkpoint");
  assert.equal(fs.readFileSync(path.join(graph, "pages/Refs.md"), "utf8"), refs);
  await shot("warm.png");
  receipt.passed.push(stage);
  receipt.outcome = "passed";
} catch (error) {
  receipt.outcome = "failed";
  receipt.capsule = { stage, expected: "moved references and embeds show their destination before and after warm restart", observed: String(error), classification: "ambiguous" };
  await shot("failure.png");
  receipt.capsule.screenshot = fs.existsSync(path.join(run, "failure.png")) ? "failure.png" : null;
  process.exitCode = 1;
} finally {
  await stop();
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify(receipt, null, 2) + "\n");
  console.log(JSON.stringify({ ...receipt, artifacts: run }));
}
