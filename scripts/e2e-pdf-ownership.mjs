// Native H2 proof: delayed PDF work stays with the graph/window generation that
// created it.  The oracle is sidecar bytes plus a real close/relaunch, never
// pixels or debounce duration alone.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "linux") throw new Error("PDF ownership native proof is Linux-only");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4540);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4541);
const TMP = path.join(os.tmpdir(), `tine-pdf-ownership-e2e-${process.pid}`);
const GRAPH_A = path.join(TMP, "graph-a");
const GRAPH_B = path.join(TMP, "graph-b");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;
const PDF = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA1IDAgUiA+PiA+PiAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAyMDUgPj4Kc3RyZWFtCkJUIC9GMSAyMCBUZiA3MiA3MjAgVGQgKFRpbmUgUERGIHZpZXdlcikgVGogRVQKQlQgL0YxIDEzIFRmIDcyIDY5MCBUZCAoU2VsZWN0IHRoaXMgdGV4dCB0byBjcmVhdGUgYSBoaWdobGlnaHQuKSBUaiBFVApCVCAvRjEgMTMgVGYgNzIgNjY4IFRkIChIaWdobGlnaHRzIHBlcnNpc3QgdG8gYXNzZXRzLzxrZXk+LmVkbiArIGFuIGhsc19fIHBhZ2UuKSBUaiBFVAoKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlMSAvVHlwZTEgL0Jhc2VGb250IC9IZWx2ZXRpY2EgPj4KZW5kb2JqCnhyZWYKMCA2CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAwOSAwMDAwMCBuIAowMDAwMDAwMDU4IDAwMDAwIG4gCjAwMDAwMDAxMTUgMDAwMDAgbiAKMDAwMDAwMDI0MSAwMDAwIG4gCjAwMDAwMDA0OTcgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA2IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlCjU2NwolJUVPRg==";

fs.rmSync(TMP, { recursive: true, force: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
const today = new Date();
const journal = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}.md`;
for (const [graph, owner] of [[GRAPH_A, "A"], [GRAPH_B, "B"]]) {
  for (const dir of ["pages", "journals", "logseq", "assets"]) {
    fs.mkdirSync(path.join(graph, dir), { recursive: true });
  }
  fs.writeFileSync(path.join(graph, "logseq", "config.edn"), "{}\n");
  fs.writeFileSync(path.join(graph, "journals", journal), `- ![Shared ${owner}](../assets/shared.pdf)\n`);
  fs.writeFileSync(path.join(graph, "assets", "shared.pdf"), Buffer.from(PDF, "base64"));
  fs.writeFileSync(
    path.join(graph, "assets", "shared.edn"),
    `{:highlights [] :extra {:page 1 :scale 1.0 :owner "${owner}"} :owner-root "${owner}"}\n`,
  );
}
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
const appData = path.join(TMP, "xdg", "data", "page.tine.Tine");
fs.mkdirSync(appData, { recursive: true });
fs.writeFileSync(path.join(appData, "tine-settings.json"), JSON.stringify({
  known_graphs: [
    { name: "graph-a", path: GRAPH_A },
    { name: "graph-b", path: GRAPH_B },
  ],
  last_graph_path: GRAPH_A,
}, null, 2));

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH_A,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
let driver;
let browser;

function startDriver() {
  driver = spawn(TD, [
    "--port", String(DRIVER_PORT),
    "--native-port", String(NATIVE_PORT),
    "--native-driver", WD,
  ], { env, stdio: ["ignore", log, log], detached: true });
}

function stopDriver() {
  try { if (driver?.pid) process.kill(-driver.pid, "SIGKILL"); } catch {}
  driver = undefined;
}

async function connect() {
  startDriver();
  await sleep(2500);
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
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
}

async function waitForNode(predicate, message, timeoutMs = 12_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = predicate();
    if (value) return value;
    await sleep(50);
  }
  throw new Error(message);
}

function sidecar(graph) {
  return path.join(graph, "assets", "shared.edn");
}

function sidecarScale(graph) {
  const text = fs.readFileSync(sidecar(graph), "utf8");
  const value = Number(text.match(/:scale\s+([0-9.]+)/)?.[1]);
  if (!Number.isFinite(value)) throw new Error(`missing sidecar scale in ${text}`);
  return value;
}

async function openSharedPdf() {
  const link = await browser.$(".pdf-link");
  await link.waitForExist({ timeout: 15_000 });
  await link.click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "shared.pdf" &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 15_000,
    timeoutMsg: "same-name PDF did not mount through the real asset link",
  });
}

async function switchGraph(graph) {
  await browser.$(".graph-switch-btn").click();
  await browser.$(".graph-switch-menu").waitForExist({ timeout: 5_000 });
  // WebKitDriver can invalidate a session when several element-attribute
  // commands race. Snapshot and activate the exact known-graph row in one
  // document command; filesystem state below remains the semantic oracle.
  const clicked = await browser.execute((target) => {
    const row = [...document.querySelectorAll(".graph-switch-row")]
      .find((candidate) => candidate.getAttribute("title") === target);
    row?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, view: window }));
    return Boolean(row);
  }, graph);
  if (!clicked) throw new Error(`known graph row missing for ${graph}`);
  await browser.waitUntil(() => browser.execute((name) =>
    document.querySelector(".graph-switch-name")?.textContent?.trim() === name,
  path.basename(graph)), {
    timeout: 40_000,
    timeoutMsg: `${path.basename(graph)} did not become the in-place graph binding`,
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
}

const receipt = { graphSwitch: {}, safeClose: {} };
try {
  await connect();
  const originalB = fs.readFileSync(sidecar(GRAPH_B));
  await openSharedPdf();
  await browser.$('button[title="Zoom in"]').click();
  const graphAScheduledZoom = Number((await browser.$(".pdf-zoom-level").getText()).replace("%", "")) / 100;

  // Switch immediately, before the ordinary four-second view-state callback.
  // The graph transaction must force A durable, unmount it, and bind B only
  // after the old generation is quiescent.
  await switchGraph(GRAPH_B);
  await browser.$(".pdf-viewer").waitForExist({ reverse: true, timeout: 10_000 });
  await waitForNode(
    () => Math.abs(sidecarScale(GRAPH_A) - graphAScheduledZoom) < 0.0001,
    "graph switch did not flush graph A's pending PDF position",
  );
  await sleep(4300); // expose any uncancelled old debounce; filesystem is the oracle
  if (!fs.readFileSync(sidecar(GRAPH_B)).equals(originalB)) {
    throw new Error("graph A's stale PDF callback changed graph B's same-name sidecar");
  }
  receipt.graphSwitch = {
    graphAScheduledZoom,
    graphAPersistedZoom: sidecarScale(GRAPH_A),
    graphBByteStable: true,
    oldViewerUnmounted: true,
  };

  // Ordinary close: create a fresh pending position, close the actual graph
  // window immediately, then relaunch and reopen the PDF.  Restored domain
  // state proves safe-close enrolled the debounce before teardown.
  await switchGraph(GRAPH_A);
  await openSharedPdf();
  await browser.$('button[title="Zoom in"]').click();
  const closeScheduledZoom = Number((await browser.$(".pdf-zoom-level").getText()).replace("%", "")) / 100;
  await browser.closeWindow();
  await waitForNode(
    () => Math.abs(sidecarScale(GRAPH_A) - closeScheduledZoom) < 0.0001,
    "safe close did not persist the pending PDF position",
  );
  try { await browser.deleteSession(); } catch {}
  browser = undefined;
  stopDriver();
  await sleep(800);

  await connect();
  await openSharedPdf();
  const relaunchedZoom = Number((await browser.$(".pdf-zoom-level").getText()).replace("%", "")) / 100;
  if (Math.abs(relaunchedZoom - closeScheduledZoom) >= 0.0001) {
    throw new Error(`relaunch restored PDF scale ${relaunchedZoom}, expected ${closeScheduledZoom}`);
  }
  receipt.safeClose = {
    closeScheduledZoom,
    persistedZoom: sidecarScale(GRAPH_A),
    relaunchedZoom,
  };
  fs.writeFileSync(path.join(ARTIFACTS, "pdf-ownership-native-receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(`PASS: PDF graph-switch and safe-close ownership are filesystem-stable: ${JSON.stringify(receipt)}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  stopDriver();
  fs.closeSync(log);
}
