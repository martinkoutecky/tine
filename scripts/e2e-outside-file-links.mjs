// GH #457: real WebKit -> Tauri -> Linux OS opener, with a recording handler.
import { spawn, spawnSync } from "node:child_process";
import { setTimeout as poll } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

ensurePrivateSessionBus();
assert.ok(process.env.DISPLAY, "Run under xvfb-run -a");
const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const artifacts = path.resolve("artifacts/gh457");
const testedCommit = spawnSync("rtk", ["git", "rev-parse", "HEAD"], { encoding: "utf8" }).stdout.trim();
fs.mkdirSync(artifacts, { recursive: true });
const fixture = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(fixture, "graph");
const document = path.join(fixture, "documents", "a report.txt");
const handoff = path.join(fixture, "handoff.jsonl");
const bin = path.join(fixture, "bin");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.mkdirSync(path.dirname(document), { recursive: true });
fs.writeFileSync(document, "external document sentinel\n");
const page = path.join(graph, "pages/Outside Links.md");
const bytes = `- [Sibling](<../documents/a report.txt>)\n- [Absolute](<${document}>)\n- [URL](file://${document.replaceAll(" ", "%20")})\n`;
fs.writeFileSync(page, bytes);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
fs.mkdirSync(bin);
fs.writeFileSync(path.join(bin, "xdg-open"), `#!/usr/bin/env node\nimport('node:fs').then(fs => fs.appendFileSync(${JSON.stringify(handoff)}, JSON.stringify(process.argv.slice(2))+String.fromCharCode(10)));\n`, { mode: 0o755 });
const xdg = path.join(fixture, "xdg");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, dir), { recursive: true });
const settings = path.join(xdg, "data", APP_ID);
fs.mkdirSync(settings, { recursive: true });
fs.writeFileSync(path.join(settings, "tine-settings.json"), JSON.stringify({ last_graph_path: graph }));
const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(xdg, "data"), XDG_CONFIG_HOME: path.join(xdg, "config"), XDG_CACHE_HOME: path.join(xdg, "cache"),
  GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1" };
const port = Number(process.env.E2E_DRIVER_PORT || 4590);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4591);
const log = fs.openSync(path.join(fixture, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
const observations = [];
const calls = () => fs.existsSync(handoff) ? fs.readFileSync(handoff, "utf8").trim().split("\n").map(JSON.parse).filter((args) => args[0] !== "--probe") : [];
try {
  const probe = spawnSync(path.join(bin, "xdg-open"), ["--probe"], { env, encoding: "utf8" });
  assert.equal(probe.status, 0, `recording opener must run: ${probe.stderr}`);
  assert.deepEqual(calls(), []);
  const deadline = Date.now() + 10_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    assert.ok(Date.now() < deadline, "driver readiness"); await poll(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await openPageByName(browser, "Outside Links");
  for (const [index, label] of ["Sibling", "Absolute", "URL"].entries()) {
    const state = await browser.execute((text) => {
      const link = [...document.querySelectorAll("a")].find((a) => a.textContent === text);
      if (!link) throw new Error(`missing link ${text}`);
      return { title: link.title, broken: link.hasAttribute("data-broken-file-link"), decoration: getComputedStyle(link).textDecorationLine };
    }, label);
    assert.equal(state.title, document); assert.equal(state.broken, false);
    assert.ok(!state.decoration.includes("line-through"));
    observations.push({ label, ...state });
    await browser.execute((text) => [...document.querySelectorAll("a")].find((a) => a.textContent === text).click(), label);
    await browser.waitUntil(() => calls().length === index + 1, { timeout: 10_000, timeoutMsg: `${label} did not reach the OS opener` });
    assert.deepEqual(calls()[index], [document]);
  }
  assert.equal(fs.readFileSync(page, "utf8"), bytes, "opening links must not rewrite the page");
  assert.equal(fs.readFileSync(document, "utf8"), "external document sentinel\n");
  fs.writeFileSync(path.join(fixture, "journey.json"), JSON.stringify({ testedCommit, observations, handoff: calls(), contentUnchanged: true }, null, 2));
  console.log(`PASS: sibling, absolute and file URL links reached the native OS opener with the same resolved path; no content changed. ${fixture}`);
} catch (error) {
  const visible = await browser?.execute(() => document.body.innerText).catch((failure) => String(failure));
  fs.writeFileSync(path.join(fixture, "failure.txt"), `Tested commit: ${testedCommit} plus uncommitted lane diff\nJourney: outside file handoff\nExpected: OS opener receives resolved path\nObserved: ${error}\nClassification: ambiguous until diagnosed\nVisible state: ${visible}\n`);
  throw error;
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch { driver.kill(); }
  fs.closeSync(log);
}
