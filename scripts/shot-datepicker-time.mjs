// Verify + screenshot the SCHEDULED/DEADLINE date picker's time row (#30): the
// `/scheduled` slash command opens the picker; "+ Add time" reveals a native time
// input (24h value regardless of the locale display) with an × to clear it, above
// the repeat row. Headless Chromium over the mock backend — a layout/logic check;
// the native time control renders per-Chromium here, per-WebKitGTK in the real app.
// (The mock is intentionally lossy and shows no scheduled chip, so we open the
// picker via the slash command rather than by clicking a chip.)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5292;
const OUT = "screenshots";
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}
async function shotPicker(page, name) {
  const dp = page.locator(".date-picker").first();
  await dp.waitFor({ timeout: 5000 });
  const box = await dp.boundingBox();
  await page.screenshot({ path: `${OUT}/${name}`, clip: { x: Math.max(0, box.x - 10), y: Math.max(0, box.y - 10), width: box.width + 20, height: box.height + 20 } });
  console.log(`wrote ${OUT}/${name}`);
}

let browser;
try {
  await waitForServer(`http://localhost:${PORT}/`);
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
  await page.goto(`http://localhost:${PORT}/`, { timeout: 15000 });
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  await page.locator(".ls-block .block-content, .ls-block").first().click({ timeout: 4000 });
  await sleep(300);
  const ta = page.locator("textarea.block-editor").first();
  await ta.waitFor({ timeout: 4000 });
  await ta.press("End");
  await ta.type("/scheduled", { delay: 20 });
  await sleep(400);
  await page.keyboard.press("Enter");
  await sleep(500);
  await shotPicker(page, "datepicker-collapsed.png");

  await page.locator(".dp-addtime").first().click({ timeout: 4000 });
  await sleep(300);
  const ti = page.locator(".dp-time-input");
  console.log("time-input type:", await ti.getAttribute("type").catch(() => "?"));
  await ti.fill("14:30", { timeout: 4000 }).catch((e) => console.log("fill err", e));
  await sleep(200);
  await shotPicker(page, "datepicker-time.png");
  console.log("DONE");
} catch (e) {
  console.error("FAIL", e);
  process.exitCode = 1;
} finally {
  if (browser) await browser.close().catch(() => {});
  server.kill();
}
