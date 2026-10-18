// GH #619 comment 6 (2026-10-18): the inline query editor's dropdowns. The Find menu's
// labels stay on one line beside their hints, and a menu opened in one query paints
// above the query block below it (wide layout: the phone sheet is its own scroll box);
// one click on the title focuses the rename field. Real components and CSS over the calm-query harness.
// Usage: node scripts/shot-gh619-query-oct18.mjs  (writes screenshots, exits non-zero on a violation)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const out = process.env.OUT ?? "artifacts/queries/gh619-oct18";
fs.mkdirSync(out, { recursive: true });
const port = 5339;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore" });
const failures = [];
const check = (ok, what) => { console.log(ok ? "ok  " : "FAIL", what); if (!ok) failures.push(what); };
let browser;
try {
  for (let i = 0; i < 150; i++) {
    try { if ((await fetch(`http://localhost:${port}/scripts/harness/calm-query.html`)).ok) break; } catch {}
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const width of [1100, 420]) {
    const page = await browser.newPage({ viewport: { width, height: 900 }, hasTouch: width === 420 });
    await page.goto(`http://localhost:${port}/scripts/harness/calm-query.html?queries=2`);
    await page.locator(".query-count").first().waitFor();
    const first = page.locator(".query-block").first();
    // One click on the title starts a rename with the caret in the field, every time: Chromium/WebView2
    // honours `autofocus` only once per document, so the second rename used to need a second click.
    for (const round of [1, 2]) {
      await first.locator(".query-title").click();
      await first.locator(".query-title-input").waitFor();
      const titleFocused = await page.waitForFunction(() => document.activeElement?.classList.contains("query-title-input"), null, { timeout: 1500 })
        .then(() => true, () => false);
      check(titleFocused, `${width}: one click on the query title focuses the rename field (rename ${round})`);
      if (!titleFocused) await first.locator(".query-title-input").focus();
      await page.keyboard.press("Escape");
      await first.locator(".query-title").waitFor();
    }
    await first.hover();
    await first.getByLabel("Query actions").click();
    await page.getByRole("button", { name: "Edit", exact: true }).click();
    await first.locator(".qc-panel .qs-sheet").waitFor();

    await first.locator(".qs-anchor-button").click();
    await first.locator(".qs-menu .qs-option").first().waitFor();
    const find = await page.evaluate(() => [...document.querySelectorAll(".qs-menu .qs-option")].map((option) => {
      const label = option.querySelector(".qs-option-label") ?? option.firstChild;
      const range = document.createRange(); range.selectNodeContents(label);
      const lines = [...range.getClientRects()].map((rect) => Math.round(rect.top));
      return { text: option.textContent, height: option.getBoundingClientRect().height, lines: new Set(lines).size };
    }));
    console.log(JSON.stringify(find));
    check(find.length >= 2 && find.every((row) => row.lines === 1), `${width}: every Find option label sits on one line`);
    await page.screenshot({ path: `${out}/find-menu-${width}.png`, fullPage: true });
    await first.locator(".qs-anchor-button").click();

    await first.getByRole("button", { name: /Add condition/ }).click();
    const menu = first.locator(".qs-menu");
    await menu.waitFor();
    const covered = await page.evaluate(() => {
      const element = document.querySelector(".qs-menu");
      const rect = element.getBoundingClientRect();
      const misses = [];
      for (const fy of [0.25, 0.5, 0.75, 0.95]) for (const fx of [0.1, 0.5, 0.9]) {
        const x = rect.left + rect.width * fx, y = rect.top + rect.height * fy;
        if (y > innerHeight - 1) continue;
        const hit = document.elementFromPoint(x, y);
        if (!hit || !element.contains(hit)) misses.push({ fx, fy, hit: hit?.className ?? null });
      }
      return { misses, rect: { top: rect.top, bottom: rect.bottom } };
    });
    console.log(JSON.stringify(covered));
    if (width >= 1000) check(covered.misses.length === 0, `${width}: the add-condition menu paints above the next query block`);
    await page.screenshot({ path: `${out}/add-condition-${width}.png`, fullPage: true });
    await page.close();
  }
} finally { await browser?.close(); server.kill(); }
if (failures.length) { console.error(`FAIL ${failures.length}: ${failures.join("; ")}`); process.exit(1); }
console.log("PASS GH #619 oct18 query dropdown layout");
