// Focused live-network Linux acceptance: run under xvfb-run -a after sourcing
// scripts/env.sh, with TINE_APP pointing to a custom-protocol build. This proves
// client identification/metadata, not installed codecs or general playback.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import { openPageByName } from "../../scripts/lib/e2e-navigation.mjs";

if (!process.env.DISPLAY || !process.env.TINE_APP) {
  throw new Error("Set TINE_APP to the native binary and run with xvfb-run -a");
}
const tmp = fs.mkdtempSync("/tmp/tine-youtube-identity-test-");
const roots = ["main", "second"].map(name => `${tmp}/${name}`);
for (const root of roots) {
  for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${root}/${dir}`, { recursive: true });
  fs.writeFileSync(`${root}/logseq/config.edn`, "{}\n");
  fs.writeFileSync(`${root}/pages/YouTube.md`, "- {{video https://youtu.be/JI-AyLv68Xs}}\n");
}
const port = Number(process.env.E2E_DRIVER_PORT || 4566);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4567);
const log = fs.openSync(`${tmp}/driver.log`, "w");
const driver = spawn(process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`,
  ["--port", String(port), "--native-port", String(nativePort), "--native-driver", "/usr/bin/WebKitWebDriver"], {
    env: { ...process.env, TINE_GRAPH: roots[0], XDG_DATA_HOME: `${tmp}/data`, XDG_CONFIG_HOME: `${tmp}/config`, XDG_CACHE_HOME: `${tmp}/cache`, WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" },
    stdio: ["ignore", log, log], detached: true,
  });
let browser;
async function identify(surface) {
  await browser.$(".nav-item").waitForExist({ timeout: 25_000 });
  await openPageByName(browser, "YouTube");
  await browser.$("iframe.embed-iframe").waitForExist({ timeout: 20_000 });
  await browser.waitUntil(async () => browser.execute(() => {
    const iframe = document.querySelector("iframe.embed-iframe");
    const player = window.YT?.get(iframe?.id);
    if (!player?.getVideoData) return false;
    if (!window.__identityProbe) {
      window.__identityProbe = { player, errors: [] };
      player.addEventListener("onError", event => window.__identityProbe.errors.push(event.data));
    }
    return !!player.getVideoData().title || window.__identityProbe.errors.includes(153);
  }), { timeout: 45_000, interval: 100, timeoutMsg: `${surface}: YouTube returned no metadata` });
  const state = await browser.execute(() => ({
    origin: location.origin,
    title: window.__identityProbe.player.getVideoData().title,
    videoId: window.__identityProbe.player.getVideoData().video_id,
    errors: window.__identityProbe.errors,
    iframe: document.querySelector("iframe.embed-iframe").src,
  }));
  console.log(JSON.stringify({ surface, ...state }));
  if (state.errors.includes(153) || !state.title?.trim() || state.videoId !== "JI-AyLv68Xs") {
    throw new Error(`${surface}: client identity failed: ${JSON.stringify(state)}`);
  }
  if (state.origin !== "tauri://localhost") throw new Error("The app origin changed");
}
try {
  await new Promise(resolve => setTimeout(resolve, 1500));
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30_000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP } } });
  await identify("main");
  const originalHandles = await browser.getWindowHandles();
  await browser.executeAsync((path, done) => {
    window.__TAURI_INTERNALS__.invoke("open_graph_window", { path }).then(() => done(null), error => done(String(error)));
  }, roots[1]).then(error => { if (error) throw new Error(error); });
  let second;
  await browser.waitUntil(async () => {
    second = (await browser.getWindowHandles()).find(handle => !originalHandles.includes(handle));
    return !!second;
  }, { timeout: 25_000, timeoutMsg: "Second graph window did not open" });
  await browser.switchToWindow(second);
  await identify("second graph window");
  console.log("PASS: native YouTube client identity in initial and later graph WebViews");
} finally {
  try { await browser?.deleteSession(); } catch (error) { console.error(String(error)); }
  try { process.kill(-driver.pid, "SIGKILL"); } catch (error) { if (error.code !== "ESRCH") throw error; }
  fs.closeSync(log);
  console.log(`Evidence: ${tmp}`);
}
