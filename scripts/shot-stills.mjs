// Extra README/website stills for features that shipped without one: the live
// `/calc` block and the colored callouts. Headless Chromium over the mock backend
// (same approach as shot-features.mjs). Usage (after `source scripts/env.sh &&
// npm run build`):  node scripts/shot-stills.mjs
// Writes screenshots/feat-calc.png and screenshots/feat-callouts.png.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5204;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  // --- calc block (lives on the default journals feed; tight element shot) ---
  try {
    await page.waitForSelector(".calc-block", { timeout: 8000 });
    const calc = page.locator(".calc-block").first();
    await calc.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await sleep(300);
    await calc.screenshot({ path: `${OUT}/feat-calc.png` });
    console.log("OK    feat-calc");
  } catch (e) { console.log("FAIL  feat-calc", String(e).split("\n")[0]); }

  // Jump to the kitchen-sink page via the quick switcher (Ctrl+K) for the callouts.
  await page.keyboard.press("Control+k");
  await sleep(400);
  await page.keyboard.type("kitchen-sink");
  await sleep(500);
  await page.keyboard.press("Enter");
  await sleep(700);

  // --- callouts (clip spanning the first few colored callouts) ---
  try {
    await page.waitForSelector(".callout", { timeout: 8000 });
    const callouts = page.locator(".callout");
    const c = Math.min(await callouts.count(), 4);
    await callouts.nth(0).evaluate((el) => el.scrollIntoView({ block: "start" }));
    await sleep(400);
    const first = await callouts.nth(0).boundingBox();
    const last = await callouts.nth(c - 1).boundingBox();
    if (first && last) {
      const pad = 14;
      await page.screenshot({
        path: `${OUT}/feat-callouts.png`,
        clip: {
          x: Math.max(0, first.x - pad),
          y: Math.max(0, first.y - pad),
          width: Math.min(820, first.width + pad * 2),
          height: last.y + last.height - first.y + pad * 2,
        },
      });
      console.log("OK    feat-callouts");
    } else {
      await callouts.nth(0).screenshot({ path: `${OUT}/feat-callouts.png` });
      console.log("OK    feat-callouts (single)");
    }
  } catch (e) { console.log("FAIL  feat-callouts", String(e).split("\n")[0]); }

  await browser.close();
} finally {
  server.kill("SIGKILL");
}
