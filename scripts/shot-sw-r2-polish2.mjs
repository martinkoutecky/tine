// Browser observation boundary: real journal feed/page and shared carry controls.
// Usage: node scripts/shot-sw-r2-polish2.mjs before|after
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import assert from "node:assert/strict";

const phase = process.argv[2] ?? "after";
const out = `artifacts/sw-r2-polish2/${phase}`;
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("rtk", ["proxy", "npx", "vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
const observations = [];
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const touch of [false, true]) {
    const page = await browser.newPage({ viewport: { width: touch ? 390 : 1280, height: 900 }, hasTouch: touch, isMobile: touch });
    await page.goto(`http://localhost:${port}/`);
    await page.locator(".page-carry-actions").first().waitFor();
    for (const surface of ["feed", "page"]) {
      const actions = page.locator(".page-carry-actions").first();
      const row = page.locator(".page-title-row").filter({ has: actions }).first();
      await page.mouse.move(1, 1);
      await page.evaluate(() => document.activeElement?.blur());
      const opacity = () => actions.evaluate(el => getComputedStyle(el).opacity);
      await delay(200);
      const resting = await opacity();
      await row.hover();
      await delay(200);
      const hovered = await opacity();
      await page.mouse.move(1, 1);
      await actions.locator("button").first().focus();
      await delay(200);
      const focused = await opacity();
      observations.push({ touch, surface, resting, hovered, focused });
      await page.screenshot({ path: `${out}/${surface}-${touch ? "touch" : "desktop"}-focus.png` });
      await page.evaluate(async () => (await import("/src/ui.ts")).setShowCarryButtons(false));
      await page.waitForFunction(() => !document.querySelector(".page-carry-actions"));
      assert.equal(await page.locator(".page-carry-actions").count(), 0, "Settings toggle hides carry controls");
      await page.evaluate(async () => (await import("/src/ui.ts")).setShowCarryButtons(true));
      await page.locator(".page-carry-actions").first().waitFor();
      if (surface === "feed") await row.locator(".journal-title").click();
    }
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  for (const row of observations) {
    assert.equal(row.resting, row.touch ? "1" : "0", `${JSON.stringify(row)}: resting visibility`);
    assert.equal(row.hovered, "1");
    assert.equal(row.focused, "1");
  }
  console.log("PASS: feed/page, title hover, keyboard focus, no-hover touch, Settings toggle");
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
