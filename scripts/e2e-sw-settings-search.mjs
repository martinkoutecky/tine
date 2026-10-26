// Built frontend, real Settings surface; mock graph backend (no native I/O).
// Usage: node scripts/e2e-sw-settings-search.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import assert from "node:assert/strict";

const out = "artifacts/sw-settings-search";
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("rtk", ["proxy", "npx", "vite", "preview", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Preview did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const width of [1280, 390]) {
    for (const locale of ["en", "zh-CN"]) {
      const page = await browser.newPage({ viewport: { width, height: 900 } });
      await page.goto(`http://localhost:${port}/`);
      await page.locator('button.icon-btn[title^="Settings"]').first().click();
      await page.locator(".settings-modal").waitFor();
      await page.locator("select.settings-select").filter({ has: page.locator('option[value="zh-CN"]') }).selectOption(locale);
      const search = page.locator(".settings-search-input");
      const fields = page.locator(".settings-pane-body .settings-field");
      await search.fill(locale === "en" ? "sticky" : "父");
      await page.locator(".settings-search-result").first().waitFor();
      assert.equal(await fields.count(), 0, `${width}/${locale}: unrelated tab controls hidden`);
      assert.equal(await search.evaluate(el => el === document.activeElement), true);
      await page.screenshot({ path: `${out}/search-${locale}-${width}.png` });
      await search.fill("no-such-setting-706");
      await page.locator(".settings-search-empty").waitFor();
      assert.equal(await fields.count(), 0);
      await search.press("Escape");
      await fields.first().waitFor();
      assert.equal(await search.inputValue(), "");
      await search.fill("diagram editors");
      await page.locator(".settings-search-result").first().click();
      await page.locator('[data-setting-label="draw.io command"]').waitFor();
      assert.equal(await page.locator(".settings-advanced-toggle").getAttribute("aria-expanded"), "true");
      assert.equal(await search.inputValue(), "");
      assert.equal(await search.evaluate(el => el === document.activeElement), true);
      await page.screenshot({ path: `${out}/advanced-${locale}-${width}.png` });
      console.log(`PASS ${width}/${locale}: matching-only, empty, clear, Advanced navigation, focus`);
      await page.close();
    }
  }
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
