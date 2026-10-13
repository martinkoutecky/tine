// The full All pages view (GH #335): sidebar section expanded, "Open the full
// list…" clicked, journals shown and sorted by day. Headless Chromium over the
// mock backend. Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-all-pages.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5211;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const wait = async (u, t = 40) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(400);
  await page.locator(".nav-section-toggle", { hasText: "ALL PAGES" }).first().click();
  await page.locator(".nav-all-pages-open").first().click();
  await page.waitForSelector(".all-pages-open", { timeout: 4000 });
  await page.locator(".all-pages-journals input").check();
  await page.locator(".all-pages-col", { hasText: "Journal date" }).click();
  await sleep(300);
  console.log("rows in DOM:", await page.locator(".all-pages-row").count(), "count label:", await page.locator(".all-pages-count").innerText());
  await page.screenshot({ path: `${OUT}/all-pages.png` });
  console.log("OK    all-pages");
  await browser.close();
} finally {
  server.kill("SIGTERM");
}
