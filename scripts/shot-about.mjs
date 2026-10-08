// Settings → About tab (GH #32): version, links, credits. Headless Chromium over
// the mock backend, both themes. Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-about.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5209;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const wait = async (u, t = 40) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const themeToggleFirst of [false, true]) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 });
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    await sleep(300);

    await page.locator('button.icon-btn[title^="Settings"]').first().click();
    await page.waitForSelector(".settings-modal", { timeout: 3000 });
    const about = page.locator(".settings-nav-item", { hasText: "About" }).first();
    await about.click();
    await page.waitForSelector(".about-tab", { timeout: 3000 });
    if (themeToggleFirst) {
      // flip to dark via the appearance tab, then back to About
      await page.locator(".settings-nav-item", { hasText: "Appearance" }).first().click();
      await page.locator(".theme-switch").first().click();
      await sleep(200);
      await about.click();
      await page.waitForSelector(".about-tab", { timeout: 3000 });
    }
    await sleep(300);
    const name = themeToggleFirst ? "about-dark" : "about-light";
    await page.locator(".settings-modal").screenshot({ path: `${OUT}/${name}.png` });
    console.log("OK   ", name);
    await page.close();
  }
  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
