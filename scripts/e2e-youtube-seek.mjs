// Real native UI journey; offline YouTube API double observes the seek boundary.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP_ID } from "./lib/app-identity.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testedCommit = execFileSync("rtk", ["git", "rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const artifactRoot = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/youtube");
fs.mkdirSync(artifactRoot, { recursive: true });
const TMP = fs.mkdtempSync(path.join(artifactRoot, "e2e-"));
const GRAPH = `${TMP}/graph`;
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const source = "- [[Video]]\n- {{youtube-timestamp 125}}\n";
fs.writeFileSync(`${GRAPH}/pages/Timestamps.md`, source);
fs.writeFileSync(`${GRAPH}/pages/Video.md`, "- {{youtube dQw4w9WgXcQ}}\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(`${TMP}/xdg/data/${APP_ID}`, { recursive: true });
fs.writeFileSync(`${TMP}/xdg/data/${APP_ID}/tine-settings.json`, JSON.stringify({ last_graph_path: GRAPH }));
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`, WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const driverPort = Number(process.env.E2E_DRIVER_PORT || 4492);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4493);
const log = fs.openSync(`${TMP}/driver.log`, "w");
const driver = spawn(process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver"), ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
const wm = process.env.E2E_WINDOW_MANAGER ? spawn(process.env.E2E_WINDOW_MANAGER, [], { env, stdio: "ignore" }) : null;
let browser;
try {
  await sleep(2500);
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || `${ROOT}/target/debug/tine` } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Timestamps");
  // Install before the first video mounts: no network/player codecs are needed.
  await browser.execute(() => {
    window.__youtubeSeek = { players: {}, seeks: [] };
    window.YT = { Player: function (id, options) {
      window.__youtubeSeek.players[id] = options.events;
      return { seekTo: (seconds, ahead) => window.__youtubeSeek.seeks.push({ id, seconds, ahead }), getCurrentTime: () => 12, destroy: () => { delete window.__youtubeSeek.players[id]; } };
    } };
    const link = [...document.querySelectorAll(".page-ref")].find((el) => el.textContent.includes("Video"));
    if (!link) throw new Error("Video page reference absent");
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));
  });
  await browser.waitUntil(() => browser.execute(() => {
    const frame = document.querySelector(".right-sidebar iframe");
    return Boolean(frame && window.__youtubeSeek.players[frame.id]);
  }), { timeout: 15_000, timeoutMsg: "sidebar YouTube embed did not register" });
  const sidebarId = await browser.execute(() => {
    const frame = document.querySelector(".right-sidebar iframe");
    window.__youtubeSeek.players[frame.id].onStateChange({ data: 1 });
    return frame.id;
  });
  // User activation runs the real rendered timestamp handler in the main pane.
  await browser.$('[data-pane-id="main"] .youtube-ts').click();
  const seeks = await browser.execute(() => window.__youtubeSeek.seeks);
  if (seeks.length !== 1 || seeks[0].id !== sidebarId || seeks[0].seconds !== 125 || seeks[0].ahead !== true) throw new Error(`expected sidebar seekTo(125,true); observed ${JSON.stringify(seeks)}`);
  if (fs.readFileSync(`${GRAPH}/pages/Timestamps.md`, "utf8") !== source) throw new Error("timestamp activation changed page bytes");
  console.log(`PASS: main-view timestamp issued seekTo(125,true) to right-sidebar player ${sidebarId}; page bytes unchanged. API boundary double; playback not certified. Artifacts: ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(`${TMP}/failure.png`); } catch {}
  fs.writeFileSync(`${TMP}/failure.txt`, `Tested base commit: ${testedCommit} (worktree diff)\nJourney: main timestamp to right-sidebar video\nExpected: sidebar seekTo(125,true)\nObserved: ${error.stack}\nClassification: ambiguous pending diagnosis; see driver.log and failure.png\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  try { wm?.kill("SIGTERM"); } catch {}
  fs.closeSync(log);
}
