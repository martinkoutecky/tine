// Native semantic proof for cached startup revocations. The registry signing
// key is intentionally absent from this repository, so the journey consumes a
// separately supplied, already-signed fixture instead of weakening production
// verification with an E2E key or bypass.
import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

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
const FIXTURE_ROOT = path.join(ROOT, "fixtures/plugin-revocation");
const fixture = {
  controlIndex: process.env.TINE_E2E_CONTROL_INDEX || path.join(FIXTURE_ROOT, "control-index.json"),
  controlSignature: process.env.TINE_E2E_CONTROL_SIGNATURE || path.join(FIXTURE_ROOT, "control-index.json.sig"),
  revokedIndex: process.env.TINE_E2E_REVOKED_INDEX || path.join(FIXTURE_ROOT, "revoked-index.json"),
  revokedSignature: process.env.TINE_E2E_REVOKED_SIGNATURE || path.join(FIXTURE_ROOT, "revoked-index.json.sig"),
  manifest: process.env.TINE_E2E_REVOKED_MANIFEST || path.join(FIXTURE_ROOT, "manifest.json"),
  wasm: process.env.TINE_E2E_REVOKED_WASM || path.join(FIXTURE_ROOT, "plugin.wasm"),
  publicKey: process.env.TINE_E2E_REGISTRY_PUBLIC_KEY || path.join(FIXTURE_ROOT, "registry-ed25519.pub.pem"),
};

for (const [name, value] of Object.entries(fixture)) {
  if (!value || !fs.existsSync(value) || !fs.statSync(value).isFile()) {
    const signingHint = name === "revokedSignature" ? "; one offline production signature is still required (see fixtures/plugin-revocation/README.md)" : "";
    throw new Error(`native plugin revocation fixture ${name} is missing at ${value}${signingHint}`);
  }
}
const controlIndexJson = fs.readFileSync(fixture.controlIndex, "utf8");
const controlSignature = fs.readFileSync(fixture.controlSignature, "utf8").trim();
const revokedIndexJson = fs.readFileSync(fixture.revokedIndex, "utf8");
const revokedSignature = fs.readFileSync(fixture.revokedSignature, "utf8").trim();
const manifestJson = fs.readFileSync(fixture.manifest, "utf8");
const wasm = fs.readFileSync(fixture.wasm);
const controlIndex = JSON.parse(controlIndexJson);
const revokedIndex = JSON.parse(revokedIndexJson);
const manifest = JSON.parse(manifestJson);
const verifyFixtureSignature = (indexJson, signature) => crypto.verify(
  null,
  Buffer.from(indexJson),
  fs.readFileSync(fixture.publicKey),
  Buffer.from(signature, "base64"),
);
if (!verifyFixtureSignature(controlIndexJson, controlSignature)) throw new Error("positive-control registry fixture signature did not verify");
if (!verifyFixtureSignature(revokedIndexJson, revokedSignature)) throw new Error("revoked registry fixture signature did not verify");
const { remote } = await import("webdriverio");
if ((controlIndex.revocations ?? []).length !== 0) throw new Error("positive-control registry fixture must contain no revocations");
const revoked = revokedIndex.revocations?.some((item) => item.id === manifest.id && item.version === manifest.version);
if (!revoked) throw new Error(`${manifest.id}@${manifest.version} is not revoked by the supplied signed registry fixture`);
const commandLabels = (manifest.contributions?.commands ?? []).map((item) => item.title);
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
const settingsPath = path.join(appData, "tine-settings.json");
function seedEnabledSettings(indexJson, signature) {
  fs.writeFileSync(settingsPath, `${JSON.stringify({
    known_graphs: [{ name: "graph", path: GRAPH }],
    last_graph_path: GRAPH,
    plugin_states: { [manifest.id]: { version: manifest.version, enabled: true } },
    "plugin-registry-index": indexJson,
    "plugin-registry-signature": signature,
  }, null, 2)}\n`);
}

async function assertNoContribution(browser, label) {
  const state = await browser.execute((id, version, kinds) => ({
    threadDecorationVisible: kinds.includes("thread-lines") && Boolean(document.querySelector(".plugin-thread-lines")),
    identity: `${id}@${version}`,
  }), manifest.id, manifest.version, decorationKinds);
  if (state.threadDecorationVisible) throw new Error(`${label} contribution became visible: ${JSON.stringify(state)}`);
  if (commandLabels.length > 0) {
    await browser.keys(["Control", "Shift", "p"]);
    await browser.$(".switcher-input").waitForExist({ timeout: 5_000 });
    const paletteText = await browser.$(".switcher").getText();
    const leaked = commandLabels.find((commandLabel) => paletteText.includes(commandLabel));
    if (leaked) throw new Error(`${label} command appeared in the command palette: ${leaked}`);
    await browser.keys(["Escape"]);
    await browser.$(".switcher").waitForExist({ reverse: true, timeout: 5_000 });
  }
}

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
async function withAppSession(label, check) {
  const sessionLog = fs.openSync(path.join(TMP, `${label}-tauri-driver.log`), "w");
  const td = spawn(TD, [
    "--port", String(DRIVER_PORT),
    "--native-port", String(NATIVE_PORT),
    "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
  ], { env, stdio: ["ignore", sessionLog, sessionLog], detached: true });
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
    await check(browser);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(sessionLog);
    await sleep(1000);
  }
}

try {
  seedEnabledSettings(controlIndexJson, controlSignature);
  await withAppSession("control", async (browser) => {
    if (decorationKinds.includes("thread-lines")) {
      await browser.$(".plugin-thread-lines").waitForExist({ timeout: 10_000 });
    }
    if (commandLabels.length > 0) {
      await browser.keys(["Control", "Shift", "p"]);
      await browser.$(".switcher-input").waitForExist({ timeout: 5_000 });
      const paletteText = await browser.$(".switcher").getText();
      const missing = commandLabels.find((label) => !paletteText.includes(label));
      if (missing) throw new Error(`positive-control command did not activate: ${missing}`);
    }
  });
  console.log(`CONTROL PASS: ${manifest.id}@${manifest.version} visibly activated under the signed empty cache`);

  seedEnabledSettings(revokedIndexJson, revokedSignature);
  await withAppSession("revoked", async (browser) => {
    await assertNoContribution(browser, "cached-revoked");

    await browser.$('[title="Settings (t s)"]').click();
    await browser.$("button=Plugins").click();
    await browser.$("button=Installed (1)").click();
    await browser.waitUntil(async () => browser.execute((id) => {
      const row = [...document.querySelectorAll(".settings-field")].find((candidate) => candidate.textContent?.includes(id));
      const toggle = row?.querySelector('[role="switch"]');
      return Boolean(row && /revoked/i.test(row.textContent ?? "") && toggle?.getAttribute("aria-checked") === "false");
    }, manifest.id), { timeout: 10_000, timeoutMsg: "cached-revoked package was not visibly disabled" });
  });

  const persisted = JSON.parse(fs.readFileSync(settingsPath, "utf8"));
  const persistedState = persisted.plugin_states?.[manifest.id];
  if (persistedState?.version !== manifest.version || persistedState.enabled !== false) {
    throw new Error(`cached revocation was not persisted disabled: ${JSON.stringify(persistedState)}`);
  }
  const envelope = persisted.plugin_registry_cache;
  if (envelope?.schemaVersion !== 1 || envelope.indexJson !== revokedIndexJson || envelope.signature !== revokedSignature) {
    throw new Error(`legacy revocation pair did not migrate to one exact signed envelope: ${JSON.stringify(envelope)}`);
  }
  if (Object.hasOwn(persisted, "plugin-registry-index") || Object.hasOwn(persisted, "plugin-registry-signature")) {
    throw new Error("legacy split registry keys survived successful atomic migration");
  }
  console.log(`PASS: ${manifest.id}@${manifest.version} stayed absent, migrated atomically, and persisted disabled when revoked`);

  // Later cache loss must not resurrect the already revoked package: durable
  // native enablement is an independent restart safety boundary.
  delete persisted.plugin_registry_cache;
  fs.writeFileSync(settingsPath, `${JSON.stringify(persisted, null, 2)}\n`);
  await withAppSession("restart-without-cache", async (browser) => {
    await assertNoContribution(browser, "restart-without-cache");
    await browser.$('[title="Settings (t s)"]').click();
    await browser.$("button=Plugins").click();
    await browser.$("button=Installed (1)").click();
    await browser.waitUntil(async () => browser.execute((id) => {
      const row = [...document.querySelectorAll(".settings-field")].find((candidate) => candidate.textContent?.includes(id));
      const toggle = row?.querySelector('[role="switch"]');
      return toggle?.getAttribute("aria-checked") === "false";
    }, manifest.id), { timeout: 10_000, timeoutMsg: "durably disabled package re-enabled after cache loss" });
  });
  console.log(`PASS: ${manifest.id}@${manifest.version} remained disabled after later cache unavailability`);
} finally {
  for (const socket of sockets) socket.destroy();
  await new Promise((resolve) => stall.close(resolve));
}
