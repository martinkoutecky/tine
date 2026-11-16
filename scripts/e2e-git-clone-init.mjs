// Explicit init and authenticated clone through the native app on a scratch graph.
// The OS folder picker alone returns a fixture path; all git IPC and HTTPS are real.
import { execFileSync, spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { remote } from "webdriverio";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const run = path.resolve(`target/artifacts/git-clone-init/native/run-${Date.now()}`);
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
// A nongit graph must live outside the checkout: discovery intentionally
// recognizes a graph nested in an ancestor repository. Bulk evidence stays in run.
const work = fs.mkdtempSync(path.join(os.tmpdir(), "tine-git-clone-init-"));
const graph = path.join(work, "graph"), copies = path.join(work, "copies"), certs = path.join(run, "certs");
fs.mkdirSync(copies, { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state", "copies", "empty-ca-dir"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
const original = Buffer.from("- native init and clone fixture\r\n");
fs.writeFileSync(path.join(graph, "pages/Alpha.md"), original);
const fixture = installAccountHome(run, run);
const env = { ...process.env, HOME: path.join(run, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", SSL_CERT_FILE: path.join(certs, "ca.pem"), SSL_CERT_DIR: path.join(run, "empty-ca-dir"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const git = (...args) => execFileSync("git", ["-C", graph, ...args], { env, encoding: "utf8" }).trim();
execFileSync("git", ["init", "--bare", "--initial-branch=main", path.join(work, "remote.git")], { env, stdio: "pipe" });
execFileSync("git", ["-C", path.join(work, "remote.git"), "config", "http.receivepack", "true"], { env });
const python = process.env.PYTHON || "python3";
const harness = path.resolve("src-tauri/src/owner/git_sync/fixture.py");
execFileSync(python, [harness, "--certs", certs], { env });
const fixtureLog = fs.openSync(path.join(run, "https-stderr.log"), "w");
const server = spawn(python, [harness, work, certs, path.join(run, "requests.jsonl"), "auth", ""], { env, stdio: ["ignore", "pipe", fixtureLog] });
const serverLines = createInterface({ input: server.stdout });
const address = await new Promise((resolve, reject) => {
  const timeout = setTimeout(() => reject(new Error("Local HTTPS fixture did not start")), 10_000);
  serverLines.once("line", line => { clearTimeout(timeout); resolve(line.trim().replace("READY ", "")); });
  server.once("exit", code => { clearTimeout(timeout); reject(new Error(`Fixture exited ${code}`)); });
});
const url = `https://127.0.0.1:${address}/remote.git`;
const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4628), nativePort = Number(process.env.E2E_NATIVE_PORT || 4629);
const target = await startWebdriverApplication(app, env, nativePort);
const driverLog = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env: target.env, stdio: ["ignore", driverLog, driverLog], detached: true });
let browser;
try {
  await sleep(2000);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  await browser.$('button.icon-btn[title^="Settings"]').click();
  await browser.waitUntil(() => browser.execute(() => {
    [...document.querySelectorAll(".settings-nav-item")].find(node => node.textContent?.trim() === "Graph")?.click();
    return !!document.querySelector("[data-git-init]");
  }), { timeout: 15_000, timeoutMsg: "Settings never offered Make this graph a git repository" });
  await browser.$("[data-git-initial-remote]").setValue(url);
  await browser.$("[data-git-init]").click();
  await browser.$("[data-git-policy]").waitForExist({ timeout: 15_000 });
  await browser.waitUntil(() => browser.execute(() => document.querySelector("[data-git-init]")?.textContent === "Finish repository setup"), { timeout: 15_000, timeoutMsg: "Init did not retain pending publication after authentication was required" });
  await browser.$("[data-git-sign-in-open]").click();
  await browser.$("[data-git-username]").setValue("token-user");
  await browser.$("[data-git-token]").setValue("fixture-token");
  await browser.$('[data-git-sign-in-form] button[type="submit"]').click();
  await browser.waitUntil(() => browser.execute(() => !document.querySelector("[data-git-sign-in-form]")), { timeout: 10_000 });
  await browser.$("[data-git-init]").click();
  await browser.waitUntil(() => browser.execute(() => !document.querySelector("[data-git-init]")), { timeout: 20_000, timeoutMsg: "Init did not publish its first commit after sign-in" });
  if (await browser.$("[data-git-policy]").getValue() !== "off") throw new Error("Init enabled automation");
  if (!fs.readFileSync(path.join(graph, "pages/Alpha.md")).equals(original)) throw new Error("Init changed page bytes");
  const ignore = fs.readFileSync(path.join(graph, ".gitignore"), "utf8");
  if (!ignore.includes(".syncthing.*") || !ignore.includes("logseq/.tine-*")) throw new Error("Init did not add ignore rules");
  if (git("log", "--format=%s") !== "Tine: initial graph") throw new Error("First commit missing or unexpected history");
  if (git("rev-parse", "--abbrev-ref", "@{upstream}") !== "origin/main") throw new Error("Init did not set upstream");
  if (git("rev-parse", "HEAD") !== execFileSync("git", ["-C", path.join(work, "remote.git"), "rev-parse", "HEAD"], { env, encoding: "utf8" }).trim()) throw new Error("The first commit did not reach the empty remote");
  if (git("show", "HEAD:pages/Alpha.md") !== original.toString().trim()) throw new Error("First commit changed page bytes");
  await browser.saveScreenshot(path.join(run, "initialized.png"));
  console.log("PASS: Settings initializes the graph, signs in and publishes one commit to the empty remote with upstream ready, unchanged pages and automation off");
  // Make clone exercise the 401 → same private token flow as well.
  await browser.$("[data-git-sign-out]").click();
  await browser.waitUntil(() => browser.execute(() => !document.querySelector("[data-git-sign-out]")), { timeout: 10_000 });
  await browser.keys("Escape");
  // Substitute only the native file chooser answer. No production test hooks.
  await browser.execute(parent => {
    const fetch = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const address = typeof input === "string" ? input : input.url;
      if (/^ipc:/.test(address) && decodeURIComponent(new URL(address).pathname).endsWith("plugin:dialog|open")) {
        return Promise.resolve(new Response(JSON.stringify(parent), { headers: { "Content-Type": "application/json", "Tauri-Response": "ok" } }));
      }
      return fetch(input, init);
    };
    const handler = window.webkit?.messageHandlers?.ipc;
    if (handler) {
      const post = handler.postMessage.bind(handler);
      handler.postMessage = data => {
        const message = JSON.parse(data);
        if (message.cmd === "plugin:dialog|open") { window.__TAURI_INTERNALS__.runCallback(message.callback, parent); return; }
        post(data);
      };
    }
  }, copies);
  await browser.$(".graph-switch-btn").click();
  await browser.execute(() => [...document.querySelectorAll(".graph-switch-menu .ctx-item")].find(node => node.textContent?.trim() === "Clone a graph from a git URL")?.click());
  await browser.$('form[aria-label="Clone a graph from a git URL"]').waitForExist({ timeout: 10_000 });
  await browser.$('input[type="url"]').setValue(url);
  await browser.$('form[aria-label="Clone a graph from a git URL"] button[type="submit"]').click();
  await browser.$('input[type="password"]').waitForExist({ timeout: 20_000 });
  await browser.$('input[autocomplete="username"]').setValue("token-user");
  await browser.$('input[type="password"]').setValue("fixture-token");
  await browser.$('form[aria-label="Clone a graph from a git URL"] button[type="submit"]').click();
  const cloned = path.join(copies, "remote");
  await browser.waitUntil(() => browser.execute(root => document.querySelector(".graph-switch-btn")?.getAttribute("title") === root, cloned), { timeout: 30_000, timeoutMsg: "Clone did not open the graph" });
  if (!fs.readFileSync(path.join(cloned, "pages/Alpha.md")).equals(original)) throw new Error("Clone changed page bytes");
  const upstream = execFileSync("git", ["-C", cloned, "rev-parse", "--abbrev-ref", "@{upstream}"], { env, encoding: "utf8" }).trim();
  if (upstream !== "origin/main") throw new Error(`Wrong clone upstream: ${upstream}`);
  await browser.$('button.icon-btn[title^="Settings"]').click();
  await browser.waitUntil(() => browser.execute(() => {
    [...document.querySelectorAll(".settings-nav-item")].find(node => node.textContent?.trim() === "Graph")?.click();
    return document.querySelector("[data-git-policy]")?.value === "off" && !!document.querySelector('[data-git-remote="origin"]');
  }), { timeout: 15_000, timeoutMsg: "Clone did not retain automation off and origin in Settings" });
  await browser.saveScreenshot(path.join(run, "cloned.png"));
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ testedCommit, workingTree: true, journey: "git clone and init", init: "passed", clone: "passed", upstream, folderPicker: "fixture answer", network: "loopback HTTPS only" }, null, 2));
  console.log("PASS: authenticated HTTPS clone opens the default branch with origin/main ready and automation off");
} catch (error) {
  await browser?.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ testedCommit, workingTree: true, journey: "git clone and init", expected: "UI initializes unchanged graph and clones via authenticated local HTTPS with upstream ready, automation off", observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  fixture.sample();
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  stopWebdriverApplication(target);
  server.kill(); serverLines.close(); fs.closeSync(driverLog); fs.closeSync(fixtureLog);
  fs.rmSync(work, { recursive: true, force: true });
}
