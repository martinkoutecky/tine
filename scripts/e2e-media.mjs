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
const MP3 = "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VVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVX/+xDE64DC0B1tx70iYE6DrPj3sAxVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVf/7EMTqgcLYHW3HvSJgRYOs+PewDFVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVV//sQxOsBwtAdb8W9gGBLA+z6mAAEVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVX/+xDE/4AHCHVz+YmAiLME6feSEAVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVQ==";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/assets/supported.mkv`, Buffer.from(MKV, "base64"));
fs.writeFileSync(`${GRAPH}/assets/supported.mp3`, Buffer.from(MP3, "base64"));
fs.writeFileSync(`${GRAPH}/pages/Media.md`, "- ![](../assets/supported.mkv)\n- ![](../assets/supported.mp3)\n");
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

  const audio = await browser.$("audio.media-embed");
  await audio.waitForExist({ timeout: 20_000 });
  await browser.waitUntil(async () => (await browser.execute(() => {
    const media = document.querySelector("audio.media-embed");
    return media ? { readyState: media.readyState, error: media.error?.code ?? 0, src: media.currentSrc || media.src } : null;
  }))?.readyState >= 1, { timeout: 20_000, timeoutMsg: "supported MP3 never loaded metadata" });
  const audioState = await browser.execute(async () => {
    const media = document.querySelector("audio.media-embed");
    const before = media.currentTime;
    await media.play();
    await new Promise((resolve) => setTimeout(resolve, 450));
    media.pause();
    return { before, after: media.currentTime, duration: media.duration, error: media.error?.code ?? 0, src: media.currentSrc || media.src };
  });
  if (audioState.error || !(audioState.src.includes("tine-media") || audioState.src.startsWith("blob:tauri")) || !(audioState.duration > 0) || !(audioState.after > audioState.before)) {
    throw new Error(`MP3 playback did not advance: ${JSON.stringify(audioState)}`);
  }
  if (!(await browser.$(".media-audio-widen").isExisting()) || !(await browser.$("audio.media-embed + .media-open-external, .media-audio-wrap .media-open-external").isExisting())) {
    throw new Error("audio playback actions are missing");
  }
  console.log(`PASS: supported MP3 loaded and played through ${audioState.src}`);

  await browser.$(".media-audio-widen").click();
  const overlay = await browser.$(".audio-overlay");
  await overlay.waitForExist({ timeout: 10_000 });
  await browser.waitUntil(async () => (await browser.execute(() => {
    const media = document.querySelector(".audio-overlay audio");
    return media ? { readyState: media.readyState, error: media.error?.code ?? 0 } : null;
  }))?.readyState >= 1, { timeout: 20_000, timeoutMsg: "expanded MP3 never loaded metadata" });
  const overlayState = await browser.execute(async () => {
    const media = document.querySelector(".audio-overlay audio");
    const before = media.currentTime;
    await media.play();
    await new Promise((resolve) => setTimeout(resolve, 450));
    media.pause();
    return { before, after: media.currentTime, duration: media.duration, error: media.error?.code ?? 0, src: media.currentSrc || media.src };
  });
  if (overlayState.error || !(overlayState.duration > 0) || !(overlayState.after > overlayState.before)) {
    throw new Error(`expanded MP3 playback did not advance: ${JSON.stringify(overlayState)}`);
  }
  await browser.$(".audio-close").click();
  await overlay.waitForExist({ reverse: true, timeout: 10_000 });
  console.log(`PASS: expanded MP3 streamed, played, and released on close through ${overlayState.src}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
