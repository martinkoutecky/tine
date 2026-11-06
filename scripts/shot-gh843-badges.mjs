// GH #843: a block's reference count and margin comment count must be the same size on the same line.
// Fails on the old CSS (different height, 1 px or more apart). Writes one enlarged crop per size.
// Usage: npm run build && node scripts/shot-gh843-badges.mjs [outDir]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.GH843_PORT || 5292);
const OUT = process.argv[2] ?? "artifacts/gh843-badges";
const SIZES = [
  { slug: "wide", width: 1440, height: 900, margin: true },
  { slug: "narrowest-margin", width: 1280, height: 800, margin: true },
  { slug: "narrow", width: 800, height: 800 },
  { slug: "phone", width: 390, height: 800, mobile: true },
];
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser, failures = [];
try {
  mkdirSync(OUT, { recursive: true });
  for (let i = 0; i < 60; i++) { try { if ((await fetch(`http://localhost:${PORT}/`)).ok) break; } catch {} await sleep(250); }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const size of SIZES) {
    const context = await browser.newContext({ viewport: { width: size.width, height: size.height }, deviceScaleFactor: 4, hasTouch: !!size.mobile, isMobile: !!size.mobile });
    const page = await context.newPage();
    await page.goto(`http://localhost:${PORT}/?regressions`);
    await page.waitForSelector(".ls-block", { timeout: 30_000 });
    await page.evaluate(() => { location.hash = ""; });
    await page.keyboard.press("Control+k");
    const input = page.locator(".switcher-input");
    if (await input.count()) {
      await input.fill("Badge alignment regression");
      await page.waitForTimeout(400);
      await page.locator(".switcher-row", { hasText: "Badge alignment regression" }).first().click();
    }
    await page.waitForSelector(".block-refs-count", { timeout: 15_000 });
    await page.waitForTimeout(800);
    const rows = await page.evaluate(() => [...document.querySelectorAll(".block-main")].flatMap((row) => {
      const refs = row.querySelector(":scope .block-refs-count"), marker = row.querySelector(":scope > .margin-marker");
      if (!refs || !marker) return [];
      const box = (el) => { const r = el.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, height: r.height, width: r.width, font: getComputedStyle(el).fontSize }; };
      return [{ text: row.textContent.slice(0, 30), refs: box(refs), marker: box(marker) }];
    }));
    console.log(size.slug, JSON.stringify(rows));
    // Too narrow for a comment margin, comments stay inline: there is no marker, only the reference count.
    if (!rows.length && size.margin) failures.push(`${size.slug}: no block shows both badges`);
    for (const row of rows) {
      if (row.refs.height !== row.marker.height) failures.push(`${size.slug}: heights ${row.refs.height} and ${row.marker.height}`);
      if (Math.abs(row.refs.top - row.marker.top) >= 0.5) failures.push(`${size.slug}: tops ${row.refs.top} and ${row.marker.top}`);
      if (row.refs.width !== row.marker.width) failures.push(`${size.slug}: widths ${row.refs.width} and ${row.marker.width}`);
      if (row.refs.font !== row.marker.font) failures.push(`${size.slug}: fonts ${row.refs.font} and ${row.marker.font}`);
    }
    await page.screenshot({ path: path.join(OUT, `${size.slug}.png`) });
    const refs = rows[0]?.refs ?? await page.locator(".block-refs-count").first().boundingBox().then((b) => ({ left: b.x, top: b.y }));
    await page.screenshot({ path: path.join(OUT, `${size.slug}-badges.png`), clip: { x: Math.max(0, refs.left - 90), y: Math.max(0, refs.top - 12), width: 150, height: 42 } });
    await context.close();
  }
} catch (error) {
  failures.push(String(error));
} finally {
  await browser?.close().catch(() => {});
  server.kill();
}
if (failures.length) { console.error(`FAIL: ${failures.join("; ")}`); process.exit(1); }
console.log("PASS: the two badges are the same size on the same line");
