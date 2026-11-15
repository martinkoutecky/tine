// Real routed query components and property edits, with synthetic engine answers.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import assert from "node:assert/strict";
const out = process.env.OUT ?? "target/artifacts/breadcrumb-switch-881/screenshots";
fs.mkdirSync(out, { recursive: true });
const port = 5341;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/scripts/harness/query-breadcrumbs.html`)).ok) break; } catch {}
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const width of [1100, 390]) {
    const page = await browser.newPage({ viewport: { width, height: 900 }, hasTouch: width === 390 });
    await page.goto(`http://localhost:${port}/scripts/harness/query-breadcrumbs.html`);
    await page.locator('.query-group [data-block-id="task"]').waitFor();
    await page.locator(".ref-breadcrumb").waitFor();
    await page.screenshot({ path: `${out}/on-${width}.png`, fullPage: true });
    await page.getByLabel("Query actions").click();
    await page.getByRole("button", { name: "View", exact: true }).click();
    const checkbox = page.getByRole("checkbox", { name: "Show breadcrumbs", exact: true });
    await checkbox.waitFor();
    assert.equal(await checkbox.isChecked(), true);
    await page.screenshot({ path: `${out}/switch-${width}.png`, fullPage: true });
    await checkbox.uncheck();
    await page.waitForFunction(() => window.breadcrumbSource().includes("tine.breadcrumbs:: false"));
    await page.locator(".ref-breadcrumb, .query-page, .query-crumb").waitFor({ state: "detached" });
    assert.match(await page.locator(".query-group").innerText(), /Buy apples/);
    await page.getByRole("button", { name: "Done", exact: true }).click();
    await page.screenshot({ path: `${out}/off-${width}.png`, fullPage: true });
    await page.getByLabel("Query actions").click();
    await page.getByRole("button", { name: "View", exact: true }).click();
    await checkbox.check();
    await page.waitForFunction(() => !window.breadcrumbSource().includes("tine.breadcrumbs"));
    await page.locator(".ref-breadcrumb").waitFor();
    await page.getByRole("button", { name: "table", exact: true }).click();
    await checkbox.waitFor({ state: "detached" });
    await page.close();
  }
  console.log("PASS routed simple query: context, property toggle, restore, table; desktop and mobile widths");
} finally { await browser?.close(); server.kill(); }
