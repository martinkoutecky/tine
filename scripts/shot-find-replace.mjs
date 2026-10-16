// In-page find & replace (GH #208, #338): the replace row on desktop, and the
// page "⋯" menu's Find in page plus the open replace row at phone width.
// Headless Chromium over the mock backend. Usage (after `source scripts/env.sh
// && npm run build`): node scripts/shot-find-replace.mjs
import { chromium, devices } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5213;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const wait = async (u, t = 40) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const [name, viewport, touch] of [["find-replace-desktop", { width: 1280, height: 800 }, false], ["find-replace-phone", { width: 390, height: 844 }, true]]) {
    const page = await browser.newPage({ viewport, deviceScaleFactor: 2, hasTouch: touch, isMobile: touch,
      userAgent: touch ? devices["Pixel 7"].userAgent : undefined });
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    await sleep(300);
    if (touch && await page.locator("[data-mobile-drawer-scrim]").isVisible()) {
      await page.locator("[data-mobile-drawer-scrim]").click({ position: { x: 380, y: 400 } });
      await sleep(400);
    }
    if (touch) {
      await page.locator("[data-page-actions-trigger]").first().click();
      await page.waitForSelector('[data-page-action-id="find-in-page"]', { timeout: 3000 });
      await page.screenshot({ path: `${OUT}/${name}-menu.png` });
      await page.locator('[data-page-action-id="find-in-page"]').click();
    } else await page.keyboard.press("Control+f");
    await page.waitForSelector(".inpage-find-input", { timeout: 3000 });
    await page.locator(".inpage-replace-toggle").click();
    await page.locator(".inpage-find-input").fill("the");
    await page.locator(".inpage-replace-input").fill("a");
    await sleep(500);
    await page.screenshot({ path: `${OUT}/${name}.png` });
    console.log("OK   ", name);
    await page.close();
  }
  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
