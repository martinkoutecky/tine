// Guide screenshots for visual review.
// Serves built dist over vite preview (http, not file://) against the mock backend.
// Usage: npm run build && node scripts/shot-guide.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5208;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 50) {
  for (let i = 0; i < tries; i += 1) {
    try {
      const r = await fetch(url);
      if (r.ok) return;
    } catch {
      // not up yet
    }
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 860 }, deviceScaleFactor: 2 });
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  const close = page.locator(".toast-close");
  if (await close.count()) {
    await close.first().click();
    await sleep(200);
  }

  await page.locator(".help-corner-btn").click();
  await page.waitForSelector(".help-menu", { timeout: 5000 });
  await page.screenshot({ path: `${OUT}/guide-help-menu.png` });

  await page.locator(".help-menu-item", { hasText: "Guide" }).first().click();
  await page.waitForSelector(".page-guide-banner", { timeout: 8000 });

  // Formulas guide page (new): capture its live computed column.
  await page.locator("a.page-ref", { hasText: "Features/Formulas" }).first().click();
  await page.waitForSelector(".sheet-table", { timeout: 8000 });
  await sleep(400);
  await page.screenshot({ path: `${OUT}/guide-formulas-light.png`, fullPage: true });

  // Back to the guide index for the Sheets shots.
  await page.locator(".help-corner-btn").click();
  await page.waitForSelector(".help-menu", { timeout: 5000 });
  await page.locator(".help-menu-item", { hasText: "Guide" }).first().click();
  await page.waitForSelector(".page-guide-banner", { timeout: 8000 });
  await page.locator("a.page-ref", { hasText: "Features/Sheets" }).first().click();
  await page.waitForSelector(".sheet-grid", { timeout: 8000 });
  await page.waitForSelector(".sheet-table", { timeout: 8000 });
  await sleep(500);
  await page.screenshot({ path: `${OUT}/guide-light.png`, fullPage: true });

  await page.click('[title^="Toggle theme"]');
  await sleep(400);
  await page.screenshot({ path: `${OUT}/guide-dark.png`, fullPage: true });

  await browser.close();
  if (errors.length) {
    console.error("ERRORS:\n" + errors.join("\n"));
    process.exit(1);
  }
  console.log(`wrote ${OUT}/guide-help-menu.png, ${OUT}/guide-formulas-light.png, ${OUT}/guide-light.png, ${OUT}/guide-dark.png`);
} finally {
  server.kill("SIGTERM");
}
