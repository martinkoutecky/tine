// GH #878: Settings adds an alternative, both keys run, and reload retains them.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { setTimeout as poll } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

if (process.argv.includes("--help")) {
  console.log('Usage: xvfb-run -a node scripts/e2e-multiple-hotkeys.mjs (TINE_APP overrides target/debug/tine)');
  process.exit(0);
}
ensurePrivateSessionBus();
assert.ok(process.env.DISPLAY, "Run under xvfb-run");
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/og-hotkeys-878/native");
fs.mkdirSync(artifacts, { recursive: true });
const fixture = fs.mkdtempSync(path.join(artifacts, "journey-"));
const graph = path.join(fixture, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(fixture, dir));
const note = "- Multiple hotkeys acceptance\n";
const config = '{:shortcuts {:ui/toggle-help ["f6" "f7"]}}\n';
fs.writeFileSync(path.join(graph, "pages/Hotkeys.md"), note);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), config);
const settingsFile = path.join(fixture, "data", APP_ID, "tine-settings.json");
fs.mkdirSync(path.dirname(settingsFile), { recursive: true });
fs.writeFileSync(settingsFile, JSON.stringify({ last_graph_path: graph }));
const home = accountHomeFixture(fixture, fixture);
const env = { ...home.env, HOME: path.join(fixture, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(fixture, "data"), XDG_CONFIG_HOME: path.join(fixture, "config"), XDG_CACHE_HOME: path.join(fixture, "cache"), XDG_STATE_HOME: path.join(fixture, "state"),
  GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1" };
const port = Number(process.env.E2E_DRIVER_PORT || 4987);
const log = fs.openSync(path.join(fixture, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "/opt/cargo", "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), journey: "Multiple hotkeys",
  expected: "Settings preserves the default plus a recorded alternative; both local and OG config alternatives run before and after reload", steps: [] };
let browser;
const shortcutRow = '[data-shortcut-id="go/search"]';
async function checkKeys(phase) {
  for (const keys of [["Control", "k"], ["F8"]]) {
    await browser.keys(keys); await browser.keys("NULL");
    await browser.$(".switcher-input").waitForDisplayed({ timeout: 10_000 });
    await browser.keys("Escape");
    await browser.$(".switcher-input").waitForExist({ reverse: true, timeout: 10_000 });
    proof.steps.push(`${phase}: ${keys.join("+")} opens search`);
  }
  for (const keys of ["F6", "F7"]) {
    await browser.keys(keys);
    await browser.$('[role="menu"][aria-label="Help"]').waitForDisplayed({ timeout: 10_000 });
    await browser.keys("Escape");
    await browser.$('[role="menu"][aria-label="Help"]').waitForExist({ reverse: true, timeout: 10_000 });
    proof.steps.push(`${phase}: OG ${keys} opens help`);
  }
}
try {
  const deadline = Date.now() + 10_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    assert.ok(Date.now() < deadline, "driver readiness"); await poll(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await openPageByName(browser, "Hotkeys");
  await assertAppLease(home);
  await browser.$('button[title^="Settings"]').click();
  await browser.$('.settings-nav-item=Keyboard shortcuts').click();
  await browser.$(`${shortcutRow} button[title="Add keybinding"]`).click();
  await browser.keys("F8");
  await browser.waitUntil(() => browser.execute(() => JSON.parse(localStorage.getItem("logseq-claude.shortcuts") || "{}")["go/search"]?.join(",") === "mod+k,f8"), { timeout: 10_000 });
  proof.steps.push("Settings records F8 alongside mod+k in local storage");
  await browser.keys("Escape");
  await checkKeys("before reload");
  await browser.refresh();
  await browser.$('button[title^="Settings"]').waitForExist({ timeout: 30_000 });
  await browser.waitUntil(() => browser.execute(() => JSON.parse(localStorage.getItem("logseq-claude.shortcuts") || "{}")["go/search"]?.join(",") === "mod+k,f8"), { timeout: 10_000 });
  await checkKeys("after reload");
  assert.equal(fs.readFileSync(path.join(graph, "pages/Hotkeys.md"), "utf8"), note);
  assert.equal(fs.readFileSync(path.join(graph, "logseq/config.edn"), "utf8"), config);
  execFileSync("import", ["-display", env.DISPLAY, "-window", "root", path.join(fixture, "hotkeys.png")], { env, timeout: 10_000 });
  fs.writeFileSync(path.join(fixture, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS multiple hotkeys: ${proof.steps.length} steps (${fixture})`);
} catch (error) {
  try { execFileSync("import", ["-display", env.DISPLAY, "-window", "root", path.join(fixture, "failure.png")], { env, timeout: 10_000 }); } catch {}
  fs.writeFileSync(path.join(fixture, "failure.json"), JSON.stringify({ ...proof, observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  home.close?.();
  home.verify();
}
