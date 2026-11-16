// Screenshot proof for D-20261019-AL2 (card hB9Zjw): the page header's alias
// group rows ("Also called" names with no page of their own, then the group's
// other real pages) at desktop width, in the right sidebar, and at 390 px.
// Usage: npm run build && node scripts/shot-alias-groups.mjs   (writes screenshots/alias-groups/)
// The mock backend's `?aliasgroups` fixture: Germany (alias:: DE, Deutschland,
// Bundesrepublik) and Bundesrepublik (its own page, alias:: BRD).
import { chromium, devices } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5241;
const OUT = "screenshots/alias-groups";
const URL = `http://localhost:${PORT}/?aliasgroups`;

mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser;
const rows = (page, scope) => page.evaluate((sel) => ({
  names: [...document.querySelectorAll(`${sel} [data-alias-names] a.page-ref`)].map((a) => a.textContent.trim()),
  pages: [...document.querySelectorAll(`${sel} [data-alias-group-pages] a.page-ref`)].map((a) => a.textContent.trim()),
}), scope);
async function openPage(page, name) {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill(name);
  const row = page.locator(".switcher-row").filter({ hasText: name }).first();
  await row.waitFor({ timeout: 5000 });
  await page.keyboard.press("Enter");
  await page.locator("h1.page-title", { hasText: name }).waitFor({ timeout: 5000 });
  await page.locator("[data-alias-group-pages]").first().waitFor({ timeout: 5000 });
}
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(URL)).ok) break; } catch {}
    if (i === 100) throw new Error("preview server did not start");
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });

  const desktop = await (await browser.newContext({ viewport: { width: 1280, height: 720 } })).newPage();
  await desktop.goto(URL);
  await desktop.waitForSelector(".page-title", { timeout: 8000 });
  await openPage(desktop, "Germany");
  assert.deepEqual(await rows(desktop, ".page-section"), { names: ["BRD", "DE", "Deutschland"], pages: ["Bundesrepublik"] });
  await desktop.screenshot({ path: `${OUT}/desktop-germany.png` });
  await desktop.locator(".page-section [data-alias-group-pages] a.page-ref", { hasText: "Bundesrepublik" }).click({ modifiers: ["Shift"] });
  await desktop.locator(".rs-item-body [data-alias-group-pages]").waitFor({ timeout: 5000 });
  assert.deepEqual(await rows(desktop, ".rs-item-body"), { names: ["BRD", "DE", "Deutschland"], pages: ["Germany"] });
  await desktop.screenshot({ path: `${OUT}/desktop-sidebar.png` });

  const phone = await (await browser.newContext({ ...devices["Pixel 7"], viewport: { width: 390, height: 844 } })).newPage();
  await phone.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
  await phone.goto(URL);
  await phone.waitForSelector(".page-title", { timeout: 8000 });
  await openPage(phone, "Bundesrepublik");
  assert.deepEqual(await rows(phone, ".page-section"), { names: ["BRD", "DE", "Deutschland"], pages: ["Germany"] });
  const overflow = await phone.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  assert.ok(overflow <= 0, `the alias rows widen the phone page by ${overflow}px`);
  await phone.screenshot({ path: `${OUT}/phone-390-bundesrepublik.png` });
  console.log(`PASS: alias group rows at desktop, sidebar and 390 px (${OUT}/)`);
} finally {
  await browser?.close();
  server.kill();
}
