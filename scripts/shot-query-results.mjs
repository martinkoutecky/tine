// QBV: real query macro on a routed synthetic Index page, desktop and phone.
// Usage: node scripts/shot-query-results.mjs (dev Vite, no production build).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import assert from "node:assert/strict";
const port = 5297;
const out = process.env.OUT ?? "screenshots";
fs.mkdirSync(out, { recursive: true });
const server = spawn(process.execPath, ["./node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: ["ignore", "pipe", "pipe"] });
let serverLog = "";
server.stdout.on("data", (chunk) => { serverLog += chunk; });
server.stderr.on("data", (chunk) => { serverLog += chunk; });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 240; i++) {
    if (server.exitCode !== null) throw new Error(`Vite exited: ${serverLog}`);
    try { if ((await fetch(`http://localhost:${port}/scripts/harness/query-results.html`)).ok) { ready = true; break; } } catch {}
    await sleep(250);
  }
  assert.ok(ready, `Vite did not start: ${serverLog}`);
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const width of [1000, 420]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    page.on("pageerror", (error) => console.error(error));
    await page.goto(`http://localhost:${port}/scripts/harness/query-results.html`);
    await page.waitForSelector(".query-summary-table");
    assert.equal(await page.locator(".query-page-link").count(), 9);
    assert.equal(await page.locator(".query-page-props").count(), 0);
    assert.equal(await page.locator(".query-summary-table").count(), 1);
    assert.equal(await page.locator('[aria-label="Clear grouping"]').count(), 2);
    const raw = await page.evaluate(() => window.__qbvRaw());
    assert.ok(!raw[0].includes("group-field"), "rendering must not persist a grouping");
    await page.screenshot({ path: `${out}/qbv-query-results-${width}.png`, fullPage: true });
    await page.locator('[aria-label="Clear grouping"]').first().click();
    await page.waitForFunction(() => !document.body.textContent.includes("Grouped by anchor"));
    assert.ok((await page.evaluate(() => window.__qbvRaw()))[1].includes("tine.group-field::"));
    assert.equal(await page.locator(".query-page-link").count(), 9);
    await page.locator(".qs-gear").first().click();
    await page.locator(".qd-trigger").click();
    await page.getByRole("button", { name: "Change", exact: true }).click();
    await page.getByRole("option", { name: "anchor", exact: true }).waitFor();
    assert.equal((await page.evaluate(() => window.__qbvRaw()))[0], raw[0], "opening the grouping picker writes nothing");
    await page.getByRole("option", { name: "anchor", exact: true }).click();
    await page.waitForFunction(() => window.__qbvRaw()[0].includes("tine.group-field:: prop:anchor"));
    assert.equal(await page.locator(".query-summary-table").count(), 1);
    await page.close();
    console.log(`ok QBV ${width}px: title rows, named grouping, sole missing group hidden, clear persisted`);
  }
} finally {
  await browser?.close();
  server.kill();
}
