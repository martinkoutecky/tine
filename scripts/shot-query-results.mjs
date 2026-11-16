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
    // GH #619 (D-20261018-Q619 Q6): each page row carries its properties as text under the title.
    assert.equal(await page.locator(".query-page-props").count(), 9);
    assert.ok((await page.locator(".query-page-props").first().textContent()).includes("source: Synthetic corpus"));
    assert.equal(await page.locator(".query-summary-table").count(), 1);
    // Calm query blocks carry no summary row: the grouping is named, never cleared from the results.
    assert.equal(await page.locator('[aria-label="Clear grouping"]').count(), 0);
    assert.equal(await page.locator(".query-grouping-clear").count(), 0);
    const raw = await page.evaluate(() => window.__qbvRaw());
    assert.ok(!raw[0].includes("group-field"), "rendering must not persist a grouping");
    await page.screenshot({ path: `${out}/qbv-query-results-${width}.png`, fullPage: true });
    // Grouping is cleared from the block's View panel (query actions -> View -> None).
    await page.locator('[aria-label="Query actions"]').nth(1).click();
    // The harness has no popover anchoring, so the menu sits at the page corner under the results: click by DOM.
    await page.getByRole("button", { name: "View", exact: true }).evaluate((button) => button.click());
    await page.locator(".qc-panel").getByRole("button", { name: "None", exact: true }).first().evaluate((button) => button.click());
    // The harness has no save pipeline, so the shown answer is not re-adopted; what a clear does is the write.
    await page.getByRole("button", { name: "Done", exact: true }).evaluate((button) => button.click());
    await page.waitForFunction(() => window.__qbvRaw()[1].includes("tine.group-field::") && !window.__qbvRaw()[1].includes("prop:anchor"));
    assert.ok((await page.evaluate(() => window.__qbvRaw()))[1].includes("tine.group-field::"));
    assert.equal(await page.locator(".query-page-link").count(), 9);
    // Opening the grouping picker writes nothing; choosing a field does.
    const cleared = (await page.evaluate(() => window.__qbvRaw()))[1];
    await page.locator('[aria-label="Query actions"]').nth(1).click();
    await page.getByRole("button", { name: "View", exact: true }).evaluate((button) => button.click());
    await page.locator(".qc-panel").getByRole("button", { name: "Change", exact: true }).first().evaluate((button) => button.click());
    await page.getByRole("option", { name: "anchor", exact: true }).waitFor();
    assert.equal((await page.evaluate(() => window.__qbvRaw()))[1], cleared, "opening the grouping picker writes nothing");
    await page.getByRole("option", { name: "anchor", exact: true }).evaluate((option) => option.click());
    await page.waitForFunction(() => window.__qbvRaw()[1].includes("tine.group-field:: prop:anchor"));
    await page.getByRole("button", { name: "Done", exact: true }).evaluate((button) => button.click());
    assert.equal(await page.locator('[aria-label="Clear grouping"]').count(), 0);
    await page.close();
    console.log(`ok QBV ${width}px: title rows with property text, named grouping with no clear button, clear and re-group persisted from the View panel`);
  }
} finally {
  await browser?.close();
  server.kill();
}
