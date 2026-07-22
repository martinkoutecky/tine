// Linux real-app proof for GH #127. The graph's `assets` entry is a symlink to
// an external directory, with the exact canonical graph/target pair pre-approved
// in disposable device settings. This exercises the real Tauri open, media read,
// and asset write paths without weakening the first-use consent component test.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4490);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4491);
const TMP = "/tmp/tine-external-assets-e2e";
const GRAPH = `${TMP}/graph`;
const EXTERNAL = `${TMP}/external-assets`;
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lO1O0QAAAABJRU5ErkJggg==";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
fs.mkdirSync(EXTERNAL, { recursive: true });
fs.symlinkSync(EXTERNAL, `${GRAPH}/assets`, "dir");
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${EXTERNAL}/pixel.png`, Buffer.from(PNG, "base64"));
fs.writeFileSync(`${GRAPH}/pages/External assets.md`, "- External image ![](../assets/pixel.png)\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[External assets]]\n");

for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
const appData = `${TMP}/xdg/data/page.tine.Tine`;
fs.mkdirSync(appData, { recursive: true });
const canonicalGraph = fs.realpathSync(GRAPH);
const canonicalAssets = fs.realpathSync(EXTERNAL);
fs.writeFileSync(`${appData}/tine-settings.json`, JSON.stringify({
  external_assets_approvals: { [canonicalGraph]: canonicalAssets },
  last_graph_path: canonicalGraph,
}, null, 2));

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
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
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  for (const selector of ["a.page-ref=External assets", "span.page-ref=External assets", "*=External assets"]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "External assets", {
    timeout: 10_000, timeoutMsg: "external-assets page did not open",
  });
  const image = await browser.$("img.inline-image");
  await image.waitForExist({ timeout: 20_000 });
  await browser.waitUntil(async () => (await image.getProperty("complete")) === true, {
    timeout: 10_000, timeoutMsg: "external asset image did not finish loading",
  });

  const write = await browser.executeAsync((graph, done) => {
    (async () => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      const loaded = await invoke("load_graph", { path: graph });
      const generation = loaded.binding_generation;
      const name = await invoke("save_asset", {
        name: "native-write.txt",
        bytesB64: btoa("approved write"),
        bindingGeneration: generation,
      });
      done({ name });
    })().catch((error) => done({ error: String(error) }));
  }, canonicalGraph);
  if (write.error || write.name !== "native-write.txt") {
    throw new Error(`native external asset write failed: ${JSON.stringify(write)}`);
  }
  if (fs.readFileSync(`${EXTERNAL}/native-write.txt`, "utf8") !== "approved write") {
    throw new Error("native write did not land in the approved external asset root");
  }
  if (!fs.lstatSync(`${GRAPH}/assets`).isSymbolicLink()) {
    throw new Error("graph assets link was unexpectedly replaced");
  }
  console.log("PASS: approved external assets opened, rendered, and accepted a native write");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
