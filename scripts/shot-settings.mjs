// Settings modal (default tab: theme / task workflow / graph / publish / keyboard
// shortcuts) over the app, with the "All pages" sidebar expanded behind it for
// context. Headless Chromium over the mock backend. Curated → docs/img/settings.png.
// Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-settings.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5207;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const wait = async (u, t = 40) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(400);

  // expand "All pages" so the sidebar lists real page names behind the modal
  const toggle = page.locator(".nav-section-toggle", { hasText: "ALL PAGES" }).first();
  if (await toggle.count()) { await toggle.click(); await sleep(250); }

  await page.locator('button.icon-btn[title^="Settings"]').first().click();
  await page.waitForSelector(".settings-modal", { timeout: 3000 });
  await sleep(300);
  await page.screenshot({ path: `${OUT}/settings.png` });
  console.log("OK    settings");

  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
