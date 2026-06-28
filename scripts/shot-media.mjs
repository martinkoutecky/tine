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
    // Headless WebKit can't decode the fixture media inline (it falls back to the
    // open-external chip), so open the overlay via the web harness hook.
    await page.evaluate(() => window.__tineOpenAudio("../assets/voice_memo.wav", "voice_memo.wav"));
    await page.waitForSelector(".audio-overlay", { timeout: 4000 });
    await sleep(900); // decode + autoplay so the playhead/time advance
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
