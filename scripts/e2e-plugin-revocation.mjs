// Native semantic proof for cached startup revocations. The registry signing
// key is intentionally absent from this repository, so the journey consumes a
// separately supplied, already-signed fixture instead of weakening production
// verification with an E2E key or bypass.
import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME
  ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver")
  : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4490);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4491);
const STALL_PORT = Number(process.env.E2E_PREVIEW_PORT || 4492);
const TMP = process.env.E2E_TMP_DIR || "/tmp/tine-plugin-revocation-e2e";
const GRAPH = path.join(TMP, "graph");
const fixture = {
  index: process.env.TINE_E2E_REVOKED_INDEX,
  signature: process.env.TINE_E2E_REVOKED_SIGNATURE,
  manifest: process.env.TINE_E2E_REVOKED_MANIFEST,
  wasm: process.env.TINE_E2E_REVOKED_WASM,
};

for (const [name, value] of Object.entries(fixture)) {
  if (!value || !fs.existsSync(value) || !fs.statSync(value).isFile()) {
    throw new Error(`native plugin revocation fixture ${name} is missing; set TINE_E2E_REVOKED_${name.toUpperCase()}`);
  }
}
const indexJson = fs.readFileSync(fixture.index, "utf8");
const signature = fs.readFileSync(fixture.signature, "utf8").trim();
const manifestJson = fs.readFileSync(fixture.manifest, "utf8");
const wasm = fs.readFileSync(fixture.wasm);
const index = JSON.parse(indexJson);
const manifest = JSON.parse(manifestJson);
const revoked = index.revocations?.some((item) => item.id === manifest.id && item.version === manifest.version);
if (!revoked) throw new Error(`${manifest.id}@${manifest.version} is not revoked by the supplied signed registry fixture`);
const commandLabels = (manifest.contributions?.commands ?? []).map((item) => item.label);
const decorationKinds = (manifest.contributions?.blockDecorations ?? []).map((item) => item.kind);
if (commandLabels.length === 0 && !decorationKinds.includes("thread-lines")) {
  throw new Error("revoked fixture must declare a command or thread-lines decoration contribution");
}

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Plugin Revocation.md"), "- Parent block\n  - Child block\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), "- open [[Plugin Revocation]]\n");

const appData = path.join(TMP, "xdg", "data", "page.tine.Tine");
const packageDir = path.join(appData, "plugins", manifest.id, manifest.version);
fs.mkdirSync(packageDir, { recursive: true });
fs.writeFileSync(path.join(packageDir, "manifest.json"), manifestJson);
fs.writeFileSync(path.join(packageDir, "plugin.wasm"), wasm);
fs.writeFileSync(path.join(appData, "tine-settings.json"), `${JSON.stringify({
  known_graphs: [{ name: "graph", path: GRAPH }],
  last_graph_path: GRAPH,
  plugin_states: { [manifest.id]: { version: manifest.version, enabled: true } },
  "plugin-registry-index": indexJson,
  "plugin-registry-signature": signature,
}, null, 2)}\n`);

// A CONNECT proxy which accepts the TLS tunnel and then never forwards bytes.
// WebKit's HTTPS request therefore reaches a real open socket but can complete
// only through the application's AbortController deadline.
const sockets = new Set();
const stall = net.createServer((socket) => {
  sockets.add(socket);
  socket.once("data", () => socket.write("HTTP/1.1 200 Connection Established\r\n\r\n"));
  socket.once("close", () => sockets.delete(socket));
});
await new Promise((resolve, reject) => {
  stall.once("error", reject);
  stall.listen(STALL_PORT, "127.0.0.1", resolve);
});
const proxy = `http://127.0.0.1:${STALL_PORT}`;
const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  HTTPS_PROXY: proxy,
  https_proxy: proxy,
  ALL_PROXY: proxy,
  all_proxy: proxy,
  NO_PROXY: "127.0.0.1,localhost",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(TMP, "tauri-driver.log"), "w");
const td = spawn(TD, [
  "--port", String(DRIVER_PORT),
  "--native-port", String(NATIVE_PORT),
  "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
], { env, stdio: ["ignore", log, log], detached: true });
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  const state = await browser.execute((id, version, kinds) => {
    return {
      threadDecorationVisible: kinds.includes("thread-lines") && Boolean(document.querySelector(".plugin-thread-lines")),
      identity: `${id}@${version}`,
    };
  }, manifest.id, manifest.version, decorationKinds);
  if (state.threadDecorationVisible) {
    throw new Error(`cached-revoked contribution became visible: ${JSON.stringify(state)}`);
  }
  if (commandLabels.length > 0) {
    await browser.keys(["Control", "Shift", "p"]);
    const palette = await browser.$(".switcher-input");
    await palette.waitForExist({ timeout: 5_000 });
    const paletteText = await browser.$(".switcher").getText();
    const leaked = commandLabels.find((label) => paletteText.includes(label));
    if (leaked) throw new Error(`cached-revoked command appeared in the command palette: ${leaked}`);
    await browser.keys(["Escape"]);
    await browser.$(".switcher").waitForExist({ reverse: true, timeout: 5_000 });
  }

  await browser.$('[title="Settings (t s)"]').click();
  await browser.$("button=Plugins").click();
  await browser.waitUntil(async () => browser.execute((id) => {
    const rows = [...document.querySelectorAll(".settings-field")];
    return rows.some((row) => row.textContent?.includes(id) && /revoked/i.test(row.textContent ?? ""));
  }, manifest.id), { timeout: 10_000, timeoutMsg: "cached-revoked package was not visibly disabled" });
  console.log(`PASS: ${manifest.id}@${manifest.version} stayed disabled with no command or decoration contribution`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
  for (const socket of sockets) socket.destroy();
  await new Promise((resolve) => stall.close(resolve));
}
