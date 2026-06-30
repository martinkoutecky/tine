// Screenshots for the media features:
//   audio-overlay.png      — the expanded audio player (waveform + skip controls)
//   asset-name-setting.png — Settings → Backups → "Asset names" format field
// The expanded audio player is opened via the web harness hook `__tineOpenAudio`
// (gated to !isTauri()): headless WebKit can't decode the fixture media inline, so
// the in-app "Expand" button never appears in the mock.
//
// Usage:  source scripts/env.sh && npm run build && node scripts/shot-media.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

// Build a short, voice-memo-shaped WAV (16-bit mono) as a data: URL, so the
// overlay's WebAudio decode draws a REAL waveform instead of the flat no-audio
// fallback. `isExternal` matches `data:`, so it's used directly (no asset serving).
function makeWavDataUrl({ seconds = 5, rate = 16000 } = {}) {
  const n = Math.floor(seconds * rate);
  const bumps = [
    [0.35, 0.9], [0.55, 0.7], [0.8, 0.85],
    [1.25, 0.6], [1.45, 0.95], [1.7, 0.5],
    [2.2, 0.8], [2.45, 0.65],
    [2.95, 0.9], [3.2, 0.75], [3.45, 0.55],
    [3.95, 0.85], [4.2, 0.6], [4.5, 0.92],
  ];
  const w = 0.075;
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    let env = 0;
    for (const [c, a] of bumps) { const d = (t - c) / w; env += a * Math.exp(-d * d); }
    env = Math.min(1, env);
    const carrier =
      0.6 * Math.sin(2 * Math.PI * 200 * t) +
      0.3 * Math.sin(2 * Math.PI * 400 * t) +
      0.35 * Math.sin(2 * Math.PI * 90 * t);
    const s = Math.max(-1, Math.min(1, env * carrier * 0.85));
    data.writeInt16LE((s * 32767) | 0, i * 2);
  }
  const h = Buffer.alloc(44);
  h.write("RIFF", 0); h.writeUInt32LE(36 + data.length, 4); h.write("WAVE", 8);
  h.write("fmt ", 12); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24); h.writeUInt32LE(rate * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34);
  h.write("data", 36); h.writeUInt32LE(data.length, 40);
  return "data:audio/wav;base64," + Buffer.concat([h, data]).toString("base64");
}

const PORT = 5198;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}
const errors = [];
try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"] });

  // --- 1. Audio overlay (open the expanded player) -----------------------
  {
    const ctx = await browser.newContext({ viewport: { width: 1180, height: 820 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".ls-block", { timeout: 8000 });
    await sleep(300);
    // Open the overlay via the web harness hook, feeding a real synthetic WAV
    // (data: URL) so the waveform actually decodes and renders.
    const wav = makeWavDataUrl();
    await page.evaluate((u) => window.__tineOpenAudio(u, "voice_memo.wav"), wav);
    await page.waitForSelector(".audio-overlay", { timeout: 4000 });
    await sleep(1300); // decode peaks + load metadata
    // Park the playhead mid-track so the played/unplayed split is visible.
    await page.evaluate(() => {
      const a = document.querySelector(".audio-overlay audio");
      if (a && a.duration) { a.pause(); a.currentTime = a.duration * 0.42; }
    });
    await sleep(500);
    await page.screenshot({ path: "screenshots/audio-overlay.png" });
    console.log("OK    audio-overlay");
    await ctx.close();
  }

  // --- 2. Settings → Backups → Asset names field -------------------------
  {
    const ctx = await browser.newContext({ viewport: { width: 960, height: 880 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".ls-block", { timeout: 8000 });
    await page.locator('button.icon-btn[title^="Settings"]').first().click();
    await page.waitForSelector(".settings-modal", { timeout: 3000 });
    await page.locator(".settings-nav-item", { hasText: "Backups" }).click();
    await sleep(250);
    await page.locator("text=Asset names").scrollIntoViewIfNeeded();
    await sleep(150);
    await page.locator(".settings-modal").screenshot({ path: "screenshots/asset-name-setting.png" });
    console.log("OK    asset-name-setting");
    await ctx.close();
  }

  await browser.close();
  console.log(errors.length ? "CONSOLE ERRORS:\n" + errors.join("\n") : "no console errors");
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  console.error(errors.join("\n"));
  server.kill("SIGKILL");
  process.exit(1);
}
