// GH #649: actual Settings markup in Soft light/dark, including hover and About.
// npm run build; node scripts/shot-soft-settings.mjs <artifact-dir> [--verify]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import assert from "node:assert/strict";

const out = process.argv[2];
const verify = process.argv.includes("--verify");
if (!out) throw new Error("artifact directory required");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("npx", ["vite", "preview", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (i === 80) throw new Error("preview unavailable");
    await sleep(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  await page.goto(`http://localhost:${port}/`);
  await page.waitForSelector(".page-title");
  await page.locator('button.icon-btn[title^="Settings"]').first().click();
  const observations = {};
  const metrics = () => page.evaluate(() => {
    const selectors = [".settings-label", ".settings-section", ".settings-hint", ".theme-gallery-name", ".theme-gallery-author", ".theme-gallery-card.selected", ".theme-gallery-card:hover", ".settings-nav-foot", ".about-version > .settings-hint", ".about-ver-num", ".about-commit", ".about-build"];
    return Object.fromEntries(selectors.map((s) => {
      const el = document.querySelector(s);
      const css = el && getComputedStyle(el);
      return [s, css && Object.fromEntries(["color", "background-color", "font-weight", "line-height", "margin-top", "border-color", "box-shadow"].map((p) => [p, css.getPropertyValue(p)]))];
    }));
  });
  for (const mode of ["light", "dark"]) {
    await page.evaluate((mode) => {
      document.documentElement.dataset.theme = mode;
      window.__tineApplyTheme("soft");
    }, mode);
    await page.locator(".settings-nav-item", { hasText: "Appearance" }).click();
    await page.waitForSelector(".theme-gallery-card");
    // Pick through the actual card so selected state is represented too.
    await page.locator(".theme-gallery-card", { has: page.locator(".theme-gallery-name", { hasText: /^Soft$/ }) }).click();
    await page.mouse.move(0, 0);
    await page.screenshot({ path: `${out}/appearance-${mode}.png` });
    observations[`${mode}-appearance`] = await metrics();
    await page.locator(".theme-gallery-card.selected").hover();
    await page.screenshot({ path: `${out}/appearance-hover-${mode}.png` });
    observations[`${mode}-hover`] = await metrics();
    if (verify) {
      const m = observations[`${mode}-appearance`];
      assert.equal(m[".settings-label"]["font-weight"], "600");
      assert.equal(m[".settings-hint"]["font-weight"], "400");
      assert.notEqual(m[".theme-gallery-author"].color, m[".settings-hint"].color);
      assert.equal(m[".settings-nav-foot"].color, m[".theme-gallery-author"].color);
      assert.equal(m[".settings-label"].color, m[".theme-gallery-name"].color);
    }
    await page.locator(".settings-nav-item", { hasText: /^About$/ }).click();
    await page.waitForSelector(".about-build");
    await page.screenshot({ path: `${out}/about-${mode}.png` });
    observations[`${mode}-about`] = await metrics();
    if (verify) {
      const m = observations[`${mode}-about`];
      const meta = observations[`${mode}-appearance`][".theme-gallery-author"].color;
      for (const s of [".about-version > .settings-hint", ".about-commit", ".about-build"]) assert.equal(m[s].color, meta, s);
    }
    // Live theme switching must remove the Soft rules.
    await page.evaluate(() => window.__tineApplyTheme("nord"));
    assert.equal(await page.locator(".settings-modal").evaluate((el) => getComputedStyle(el).getPropertyValue("--soft-settings-meta").trim()), "");
  }
  writeFileSync(`${out}/metrics.json`, JSON.stringify(observations, null, 2) + "\n");
  console.log(`Settings light/dark, selected hover, About and live Nord switch: ${verify ? "verified" : "captured"} in ${out}`);
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
