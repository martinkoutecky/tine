// CONFLICT-INVALIDINPUT: use the routed page's real conflict review and save path.
import "./lib/e2e-auto-account-home.mjs";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "target/artifacts/conflict");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const mine = "status:: local\n- shared\n- local text\n";
const theirs = "status:: remote\n- shared\n- remote text\n";
const file = path.join(graph, "pages/Conflict Demo.md");
const copy = path.join(graph, "pages/Conflict Demo.sync-conflict-20261029-135112-MAGHY7C.md");
fs.writeFileSync(file, mine);
fs.writeFileSync(copy, theirs);
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4886);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex") };
let browser, stage = "startup";
const shot = name => {
  try { execFileSync("import", ["-window", "root", path.join(out, name)], { env, timeout: 10000 }); } catch {}
};
try {
  await sleep(1200);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, "conflict-invalidinput") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Conflict Demo");
  await browser.$(".page-conflict-preblock-choice").waitForExist({ timeout: 20000 });
  const click = async prefix => assert.equal(await browser.execute(prefix => {
    const button = [...document.querySelectorAll(".page-conflict button")]
      .find(el => el.textContent.trim().startsWith(prefix));
    button?.click();
    return Boolean(button);
  }, prefix), true, `missing conflict action ${prefix}`);

  stage = "mixed choices explain the scalar property clash and preserve both files";
  await click("Apply resolution");
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".toast")]
    .some(el => el.textContent.includes("status differs"))), { timeout: 15000,
    timeoutMsg: "property refusal did not explain which property needs a choice" });
  assert.equal(fs.readFileSync(file, "utf8"), mine);
  assert.equal(fs.readFileSync(copy, "utf8"), theirs);
  shot("property-refusal.png");

  stage = "Keep sync copy includes properties and applies through guarded storage";
  await click("Keep Sync copy");
  assert.equal(await browser.$(".page-conflict-preblock-choice").getValue(), "theirs");
  await click("Apply resolution");
  await browser.waitUntil(() => !fs.existsSync(copy) && fs.readFileSync(file, "utf8").includes("status:: remote"),
    { timeout: 20000, timeoutMsg: "whole-copy resolution did not persist its properties and retire the copy" });
  const resolved = fs.readFileSync(file, "utf8");
  assert.ok(resolved.includes("remote text"));
  assert.ok(!resolved.includes("local text"));
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".page-conflict")), { timeout: 10000 });
  const recoverable = dir => fs.existsSync(dir) && fs.readdirSync(dir, { withFileTypes: true }).some(entry => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? recoverable(file) : fs.readFileSync(file, "utf8") === theirs;
  });
  assert.ok(recoverable(path.join(graph, "logseq/.tine-trash")), "reviewed copy is not recoverable");
  shot("resolved.png");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify({ ...proof, outcome: "PASS", resolved }, null, 2));
  console.log(`PASS CONFLICT-INVALIDINPUT: ${out}`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Property clash explains the choice with both files intact; Keep sync copy applies its blocks and properties and keeps the copy recoverable",
    observed: String(error.stack), classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
