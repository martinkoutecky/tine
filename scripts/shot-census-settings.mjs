import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
const out = "artifacts/sw-census-settings";
mkdirSync(out, { recursive: true });
const server = spawn("rtk", ["proxy", "npx", "vite", "preview", "--port", "5218", "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 80; i++) { try { if ((await fetch("http://localhost:5218")).ok) break; } catch {} await sleep(250); }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const mode of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 1000 }, colorScheme: mode });
    await page.goto("http://localhost:5218");
    await page.locator('button.icon-btn[title^="Settings"]').first().click();
    await page.locator(".settings-modal").waitFor();
    await page.locator(".settings-nav-item", { hasText: "Appearance" }).click();
    await page.locator(".theme-opt").nth(mode === "light" ? 0 : 2).click();
    await sleep(300);
    const body = page.locator(".settings-pane-body");
    console.log(mode, await body.locator('button:not(.theme-gallery-card):not(.settings-advanced-toggle), select, input:not([type="file"])').count(), "controls excluding gallery cards and disclosure");
    console.log(mode, await body.locator(".settings-field, .settings-row").count(), "setting rows");
    await body.evaluate((element) => { element.scrollTop = 0; });
    await page.locator(".settings-modal").screenshot({ path: `${out}/${process.argv[2] ?? "after"}-${mode}.png` });
    await body.evaluate((element) => { element.scrollTop = element.scrollHeight; });
    await page.locator(".settings-modal").screenshot({ path: `${out}/${process.argv[2] ?? "after"}-${mode}-bottom.png` });
    await page.close();
  }
} finally { await browser?.close(); server.kill("SIGTERM"); }
