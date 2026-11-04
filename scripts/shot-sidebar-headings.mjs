// Screenshot and geometry proof for GH #704 (comment 3, point 3): the left
// sidebar's first-level section headings (Open tabs, Favorites, Recent, All
// pages, Namespaces) share one font size. "All pages" is a button and used to
// reset its font to the inherited body size, so it was visibly larger.
// Usage: npm run build && node scripts/shot-sidebar-headings.mjs   (writes screenshots/sidebar-headings/)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5239;
const OUT = "screenshots/sidebar-headings";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${PORT}/?regressions`)).ok) break; } catch {}
    if (i === 100) throw new Error("preview server did not start");
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  await page.goto(`http://localhost:${PORT}/?regressions`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  // Visit a page so Recent has a row, and open a second tab so Open tabs shows.
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("kitchen-sink");
  const row = page.locator(".switcher-row")
    .filter({ has: page.locator(".switcher-kind", { hasText: /^page$/ }) })
    .filter({ hasText: "kitchen-sink" }).first();
  await row.waitFor({ timeout: 5000 });
  await row.dispatchEvent("mousedown", { button: 0, ctrlKey: true, bubbles: true, cancelable: true });
  await sleep(150);
  await page.keyboard.press("Escape");
  const sidebar = page.locator(".left-sidebar-inner").first();
  if (!(await page.locator(".nav-all-pages-open").isVisible())) {
    await page.locator('header.topbar button[title^="Toggle sidebar"]').click();
  }
  await page.locator(".nav-all-pages-open").waitFor({ timeout: 5000 });
  const headings = await page.locator(".left-sidebar-inner .nav-section-header").evaluateAll((els) => els.map((el) => {
    const style = getComputedStyle(el);
    return { text: el.textContent.trim(), size: style.fontSize, weight: style.fontWeight, family: style.fontFamily, spacing: style.letterSpacing };
  }));
  await sidebar.screenshot({ path: `${OUT}/sidebar-1280.png` });
  console.log(JSON.stringify(headings, null, 1));
  assert.ok(headings.length >= 3, `expected several section headings, found ${headings.length}`);
  for (const key of ["size", "weight", "family", "spacing"]) {
    assert.equal(new Set(headings.map((h) => h[key])).size, 1, `the section headings differ in ${key}: ${JSON.stringify(headings.map((h) => [h.text, h[key]]))}`);
  }
  console.log(`PASS sidebar headings share one font: ${headings[0].size}`);
} finally {
  await browser?.close();
  server.kill();
}
