// Linux real-app media regression. Uses a tiny VP8-in-Matroska fixture so the
// assertion covers the range-aware native protocol and a codec WebKitGTK is
// expected to support, rather than depending on a private graph or H.264.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4470);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4471);
const TMP = "/tmp/tine-media-e2e";
const GRAPH = `${TMP}/graph`;
const MKV = "GkXfo6NChoEBQveBAULygQRC84EIQoKIbWF0cm9za2FCh4EEQoWBAhhTgGcBAAAAAAACjRFNm3TAv4T4CIkiTbuLU6uEFUmpZlOsgaFNu4tTq4QWVK5rU6yB8U27jFOrhBJUw2dTrIIBPU27jFOrhBxTu2tTrIICcewBAAAAAAAAUwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFUmpZsu/hNaSwIIq17GDD0JATYCNTGF2ZjU5LjI3LjEwMFdBjUxhdmY1OS4yNy4xMDBzpJB2SdMq9kV9sBe0q/Q7f23RRImIQI9AAAAAAAAWVK5rx7+EAvZyEq4BAAAAAAAAONeBAXPFiJsCaRugDtDZnIEAIrWcg3VuZIiBAIaFVl9WUDiDgQEj44OEC+vCAOCJsIEguoEYmoECElTDZ0CDv4Q1GwJTc3OgY8CAZ8iaRaOHRU5DT0RFUkSHjUxhdmY1OS4yNy4xMDBzc9djwItjxYibAmkboA7Q2WfIoUWjh0VOQ09ERVJEh5RMYXZjNTkuMzcuMTAwIGxpYnZweGfIokWjiERVUkFUSU9ORIeUMDA6MDA6MDEuMDAwMDAwMDAwAAAfQ7Z1QKW/hJsYIPDngQCjvoEAAIAQAwCdASogABgAAEcIhYWIhYSIAgICdaoD+AP6AghZDL0A/v1u8//jmTcwxP+Obf/xYTwOKMj/8VEAo5WBAMgAsQEAARAQABgAGFgv9AAIcACjlYEBkACxAQABEBAAGAAYWC/0AAhwAKOVgQJYALEBAAEQEAAYABhYL/QACHAAo5WBAyAAsQEAARAQABgAGFgv9AAIcAAcU7trl7+EKRq2a7uPs4EAt4r3gQHxggHG8IEJ";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/assets/supported.mkv`, Buffer.from(MKV, "base64"));
fs.writeFileSync(`${GRAPH}/pages/Media.md`, "- ![](../assets/supported.mkv)\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Media]]\n");

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
  for (const selector of ["a.page-ref=Media", "span.page-ref=Media", "*=Media"]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  const video = await browser.$("video.media-embed");
  await video.waitForExist({ timeout: 20_000 });
  await browser.waitUntil(async () => (await browser.execute(() => {
    const media = document.querySelector("video.media-embed");
    return media ? { readyState: media.readyState, error: media.error?.code ?? 0, src: media.currentSrc || media.src } : null;
  }))?.readyState >= 1, { timeout: 20_000, timeoutMsg: "supported MKV never loaded metadata" });
  const state = await browser.execute(async () => {
    const media = document.querySelector("video.media-embed");
    const before = media.currentTime;
    await media.play();
    await new Promise((resolve) => setTimeout(resolve, 450));
    media.pause();
    return { before, after: media.currentTime, duration: media.duration, error: media.error?.code ?? 0, src: media.currentSrc || media.src };
  });
  if (state.error || !(state.src.includes("tine-media") || state.src.startsWith("blob:tauri")) || !(state.duration > 0) || !(state.after > state.before)) {
    throw new Error(`MKV playback did not advance: ${JSON.stringify(state)}`);
  }
  console.log(`PASS: supported MKV loaded and played through ${state.src}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
