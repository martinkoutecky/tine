// The real query components/CSS, with synthetic answers for visual review.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import assert from "node:assert/strict";
const out = process.env.OUT ?? "artifacts/queries/screenshots";
fs.mkdirSync(out, { recursive: true });
const port = 5337;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/scripts/harness/calm-query.html`)).ok) break; } catch {}
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const width of [1100, 420]) {
    const page = await browser.newPage({ viewport: { width, height: 900 }, hasTouch: width === 420 });
    await page.goto(`http://localhost:${port}/scripts/harness/calm-query.html`);
    await page.locator(".query-count").waitFor();
    await page.locator('.query-group [data-block-id="task-0"]').waitFor();
    assert.equal(await page.locator(".qs-sentence").count(), 0);
    await page.mouse.move(0, 0);
    await page.screenshot({ path: `${out}/rest-${width}.png`, fullPage: true });
    await page.locator(".query-block").hover();
    await page.screenshot({ path: `${out}/hover-${width}.png`, fullPage: true });
    const actions = page.getByLabel("Query actions");
    await actions.focus(); await page.keyboard.press("Enter");
    await page.getByRole("button", { name: "Edit", exact: true }).click();
    assert.equal(await page.locator(".qc-panel .qs-sheet").count(), 1);
    await page.screenshot({ path: `${out}/edit-${width}.png`, fullPage: true });
    await page.getByRole("button", { name: "Done", exact: true }).click();
    await actions.click(); await page.getByRole("button", { name: "View", exact: true }).click();
    await page.getByRole("button", { name: "table", exact: true }).click();
    await page.locator(".sheet-table").waitFor();
    if (await page.getByRole("button", { name: "Done", exact: true }).count()) await page.getByRole("button", { name: "Done", exact: true }).click();
    await page.screenshot({ path: `${out}/table-${width}.png`, fullPage: true });
    await page.close();
  }
  console.log("PASS calm query rest, hover, inline edit and table at desktop and touch width");
} finally { await browser?.close(); server.kill(); }
