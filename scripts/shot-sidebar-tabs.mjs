// Browser geometry and screenshot proof for GH #704/#705 under D-20261018-W1:
// the sidebar's Tabs section lists the open tabs at every width, and the top
// bar's tab strip appears when there is room (>= 640 px) on any device.
// Usage: npm run build && node scripts/shot-sidebar-tabs.mjs   (writes screenshots/sidebar-tabs/)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5237;
const OUT = "screenshots/sidebar-tabs";
const PAGES = ["kitchen-sink", "org-sink", "Sheets demo"];
const CASES = [
  { name: "phone-390", width: 390, height: 844, touch: true, platform: "android", strip: false },
  { name: "tablet-820", width: 820, height: 1180, touch: true, platform: "ios", strip: true },
  { name: "desktop-1280", width: 1280, height: 800, touch: false, platform: undefined, strip: true },
  { name: "desktop-narrow-500", width: 500, height: 800, touch: false, platform: undefined, strip: false },
];

mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${PORT}/?regressions`)).ok) break; } catch {}
    if (i === 100) throw new Error("preview server did not start");
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const c of CASES) {
    const context = await browser.newContext({ viewport: { width: c.width, height: c.height }, hasTouch: c.touch, isMobile: false });
    const page = await context.newPage();
    await page.addInitScript(({ platform }) => {
      if (platform) globalThis.__TINE_PLATFORM__ = platform;
      localStorage.setItem("logseq-claude.sidebarOpen", "0");
    }, { platform: c.platform });
    await page.goto(`http://localhost:${PORT}/?regressions`);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    // Open background tabs the way the Search chord does (Ctrl+click), which
    // works with a mouse or a keyboard on every device class.
    for (const name of PAGES) {
      await page.keyboard.press("Control+k");
      await page.locator(".switcher-input").fill(name);
      const row = page.locator(".switcher-row")
        .filter({ has: page.locator(".switcher-kind", { hasText: /^page$/ }) })
        .filter({ hasText: name }).first();
      await row.waitFor({ timeout: 5000 });
      await row.dispatchEvent("mousedown", { button: 0, ctrlKey: true, bubbles: true, cancelable: true });
      await sleep(150);
      await page.keyboard.press("Escape");
    }
    const strip = await page.locator("header.topbar .tab-bar").count();
    assert.equal(strip > 0, c.strip, `${c.name}: top bar tab strip ${c.strip ? "shown" : "hidden"} at ${c.width}px`);
    // The sidebar (a drawer on a phone-width window) with the Tabs section.
    await page.locator('header.topbar button[title^="Toggle sidebar"]').click();
    await page.locator('[data-sidebar-section="tabs"]').waitFor({ timeout: 5000 });
    const rows = page.locator('[data-sidebar-section="tabs"] .nav-tab');
    assert.equal(await rows.count(), PAGES.length + 1, `${c.name}: every tab is listed`);
    assert.equal(await page.locator('[data-sidebar-section="tabs"] .nav-tab.active').count(), 1, `${c.name}: one active row`);
    await page.screenshot({ path: `${OUT}/${c.name}-sidebar-tabs.png` });
    // Switch to the last tab from the list; it becomes the active row.
    await rows.last().locator(".nav-tab-main").click();
    if (c.width < 640) {
      // A drawer closes after navigation; reopen it to read the list again.
      await page.locator('[data-mobile-drawer-panel="left"]').waitFor({ state: "detached", timeout: 5000 });
      await page.locator('header.topbar button[title^="Toggle sidebar"]').click();
    }
    await page.locator('[data-sidebar-section="tabs"]').waitFor({ timeout: 5000 });
    const activeIndex = await rows.evaluateAll((els) => els.findIndex((el) => el.classList.contains("active")));
    assert.equal(activeIndex, PAGES.length, `${c.name}: the clicked row is the active one`);
    console.log(`${c.name}: strip=${c.strip} rows=${PAGES.length + 1} active=${activeIndex}`);
    await context.close();
  }
} finally {
  await browser?.close();
  server.kill();
}
