// Native semantic proof: a delayed plugin response belongs to the exact graph
// generation/editor that invoked it, even when graph B has identical UUID/raw.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "/aux/koutecky/logseq/.toolchain/cargo", "bin", "tauri-driver");
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4494);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4495);
const TMP = process.env.E2E_TMP_DIR || "/tmp/tine-plugin-graph-ownership-e2e";
const A = path.join(TMP, "graph-a");
const B = path.join(TMP, "graph-b");
const XDG = path.join(TMP, "xdg");
const FIXTURE = path.join(ROOT, "fixtures/plugin-graph-ownership");
const manifest = JSON.parse(fs.readFileSync(path.join(FIXTURE, "manifest.json"), "utf8"));
const wasm = fs.readFileSync(path.join(FIXTURE, "plugin.wasm"));
const { remote } = await import("webdriverio");

const now = new Date();
const JOURNAL = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}.md`;
const sourceBytes = "- same raw\n  id:: shared-id\n";
const resultBytes = "- plugin result\n  id:: shared-id\n";
const journalPath = (root) => path.join(root, "journals", JOURNAL);

fs.rmSync(TMP, { recursive: true, force: true });
for (const root of [A, B]) {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(root, dir), { recursive: true });
  fs.writeFileSync(path.join(root, "logseq", "config.edn"), "{}\n");
  fs.writeFileSync(journalPath(root), sourceBytes);
}
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(XDG, dir), { recursive: true });
const appData = path.join(XDG, "data", "page.tine.Tine");
const packageDir = path.join(appData, "plugins", manifest.id, manifest.version);
fs.mkdirSync(packageDir, { recursive: true });
fs.writeFileSync(path.join(packageDir, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
fs.writeFileSync(path.join(packageDir, "plugin.wasm"), wasm);
fs.writeFileSync(path.join(appData, "tine-settings.json"), `${JSON.stringify({
  known_graphs: [{ name: "graph-a", path: A }, { name: "graph-b", path: B }],
  last_graph_path: A,
  plugin_states: { [manifest.id]: { version: manifest.version, enabled: true } },
}, null, 2)}\n`);

const env = {
  ...process.env,
  TINE_GRAPH: A,
  XDG_DATA_HOME: path.join(XDG, "data"),
  XDG_CONFIG_HOME: path.join(XDG, "config"),
  XDG_CACHE_HOME: path.join(XDG, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const logPath = path.join(TMP, "tauri-driver.log");
const log = fs.openSync(logPath, "w");
const driver = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", WD], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$('[data-block-id="shared-id"] .block-content').waitForExist({ timeout: 20_000 });
  await browser.$('[data-block-id="shared-id"] .block-content').click();
  await browser.$('[data-block-id="shared-id"] textarea.block-editor').waitForExist({ timeout: 5_000 });

  const raced = await browser.execute(async (graphB) => {
    const editor = document.querySelector('[data-block-id="shared-id"] textarea.block-editor');
    if (!(editor instanceof HTMLTextAreaElement)) return "missing-editor";
    editor.dispatchEvent(new KeyboardEvent("keydown", {
      key: "g", code: "KeyG", ctrlKey: true, altKey: true, bubbles: true, cancelable: true,
    }));
    document.querySelector(".graph-switch-btn")?.click();
    await Promise.resolve();
    const row = [...document.querySelectorAll(".graph-switch-row")].find((candidate) => candidate.getAttribute("title") === graphB);
    if (!(row instanceof HTMLElement)) return "missing-graph-row";
    row.click();
    return "started";
  }, B);
  if (raced !== "started") throw new Error(`could not start graph-switch race: ${raced}`);
  await browser.waitUntil(async () => (await browser.$(".graph-switch-name").getText()) === "graph-b", {
    timeout: 20_000, timeoutMsg: "graph B did not bind",
  });
  await sleep(600);
  if (fs.readFileSync(journalPath(A), "utf8") !== sourceBytes) throw new Error("stale response changed graph A after transition began");
  if (fs.readFileSync(journalPath(B), "utf8") !== sourceBytes) throw new Error("stale graph-A response changed equal graph-B bytes");
  const staleUi = await browser.$('[data-block-id="shared-id"] .block-content').getText();
  if (!staleUi.includes("same raw")) throw new Error(`graph B routed state changed: ${staleUi}`);

  await browser.$('[data-block-id="shared-id"] .block-content').click();
  await browser.$('[data-block-id="shared-id"] textarea.block-editor').waitForExist({ timeout: 5_000 });
  await browser.execute(() => {
    document.querySelector('[data-block-id="shared-id"] textarea.block-editor')?.dispatchEvent(new KeyboardEvent("keydown", {
      key: "g", code: "KeyG", ctrlKey: true, altKey: true, bubbles: true, cancelable: true,
    }));
  });
  await browser.waitUntil(() => fs.readFileSync(journalPath(B), "utf8") === resultBytes, {
    timeout: 10_000, timeoutMsg: "current graph-B plugin result did not persist",
  });

  console.log(JSON.stringify({
    candidate: process.env.TINE_E2E_CANDIDATE || "unstamped",
    roots: ["A", "B"],
    staleABytes: sourceBytes,
    staleBBytes: sourceBytes,
    currentBBytes: resultBytes,
    graphBStayedUnchangedUnderStaleAResult: true,
    currentGraphPositiveControlPersisted: true,
  }, null, 2));
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
