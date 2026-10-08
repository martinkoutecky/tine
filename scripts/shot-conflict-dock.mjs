// Self-verification shots for the conflict dock: the full panel at the top of a
// long page, the slim pinned bar once the panel scrolls out of view, and the
// unrolled-in-place sheet, at desktop and phone widths.
// Usage: npm run build && node scripts/shot-conflict-dock.mjs   (needs scripts/env.sh for Chromium)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5218;
const OUT = process.env.SHOT_DIR || "/tmp/shot-conflict-dock";
mkdirSync(OUT, { recursive: true });
const server = spawn("./node_modules/.bin/vite", ["preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url) {
  for (let i = 0; i < 80; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      /* not up yet */
    }
    await sleep(250);
  }
  throw new Error("server did not start");
}

async function openConflictPage(page, { phone = false } = {}) {
  await page.goto(`http://localhost:${PORT}/?conflicts`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.waitForSelector(".conflict-queue-badge", { timeout: 5000 });
  // The sticky Guide announcement toast overlays the sidebar footer on a phone.
  await page.evaluate(() => document.querySelector(".toast-close")?.click());
  await sleep(300);
  await page.locator(".conflict-queue-badge").click();
  await sleep(500);
  if (process.env.SHOT_DEBUG) await page.screenshot({ path: `${OUT}/debug-${phone ? "phone" : "desktop"}.png` });
  await page.locator(".conflict-overview-open").first().click();
  await sleep(400);
  await page.waitForSelector(".page-conflict", { state: "attached", timeout: 5000 });
}
const scrollPane = (page, top) =>
  page.evaluate((y) => {
    document.querySelector(".main-content").scrollTop = y;
  }, top);

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu"] });
  const errors = [];

  const desktop = await browser.newPage({ viewport: { width: 1180, height: 800 }, deviceScaleFactor: 2 });
  desktop.on("pageerror", (e) => errors.push(String(e)));
  await openConflictPage(desktop);
  await scrollPane(desktop, 0);
  await sleep(400);
  if (await desktop.locator(".page-conflict-dock").count()) throw new Error("dock bar visible while the panel is at the top");
  await desktop.screenshot({ path: `${OUT}/top.png` });

  await scrollPane(desktop, 2500);
  await sleep(500);
  await desktop.waitForSelector(".page-conflict-dockbar", { timeout: 5000 });
  await desktop.screenshot({ path: `${OUT}/bar.png` });

  await desktop.locator(".page-conflict-dockbar").click();
  await sleep(400);
  await desktop.waitForSelector(".page-conflict-sheet .page-conflict", { timeout: 5000 });
  await desktop.screenshot({ path: `${OUT}/sheet.png` });

  await scrollPane(desktop, 0);
  await sleep(500);
  if (await desktop.locator(".page-conflict-dock").count()) throw new Error("dock still present after scrolling back to the top");
  await desktop.waitForSelector(".page-conflict-slot .page-conflict", { timeout: 5000 });

  const phone = await browser.newPage({ viewport: { width: 400, height: 900 }, deviceScaleFactor: 2 });
  phone.on("pageerror", (e) => errors.push(String(e)));
  await openConflictPage(phone, { phone: true });
  await scrollPane(phone, 3000);
  await sleep(500);
  await phone.waitForSelector(".page-conflict-dockbar", { timeout: 5000 });
  await phone.screenshot({ path: `${OUT}/phone-bar.png` });
  await phone.locator(".page-conflict-dockbar").click();
  await sleep(400);
  await phone.waitForSelector(".page-conflict-sheet .page-conflict", { timeout: 5000 });
  await phone.screenshot({ path: `${OUT}/phone-sheet.png` });

  await browser.close();
  server.kill("SIGKILL");
  if (errors.length) {
    console.error("PAGE ERRORS:\n" + errors.join("\n"));
    process.exit(1);
  }
  console.log(`shots: ${OUT}/{top,bar,sheet,phone-bar,phone-sheet}.png`);
} catch (e) {
  server.kill("SIGKILL");
  console.error(e);
  process.exit(1);
}
