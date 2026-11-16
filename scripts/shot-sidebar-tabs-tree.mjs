// Phone-width screenshot proof for GH #704 v2 (the sidebar Tabs tree): on a
// touch phone a long-press drag to the right nests a tab, the disclosure
// triangle collapses the group, and the rows indent by depth.
// Usage: npm run build && node scripts/shot-sidebar-tabs-tree.mjs   (writes screenshots/sidebar-tabs-tree/)
// Touch is driven as synthetic PointerEvents (pointerType "touch"), so this
// proves the hold-then-drag logic and the layout, not the device's native
// scroll arbitration.
import { chromium, devices } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5239;
const OUT = "screenshots/sidebar-tabs-tree";
const PAGES = ["kitchen-sink", "org-sink", "Sheets demo"];

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
  const pixel = devices["Pixel 7"];
  const context = await browser.newContext({ ...pixel, viewport: { width: 390, height: 844 } });
  const page = await context.newPage();
  await page.addInitScript(() => {
    globalThis.__TINE_PLATFORM__ = "android";
    localStorage.setItem("logseq-claude.sidebarOpen", "0");
  });
  await page.goto(`http://localhost:${PORT}/?regressions`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
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
  const openDrawer = async () => {
    await page.locator('header.topbar button[title^="Toggle sidebar"]').click();
    await page.locator('[data-sidebar-section="tabs"]').waitFor({ timeout: 5000 });
  };
  await openDrawer();
  const rows = page.locator('[data-sidebar-section="tabs"] .nav-tab');
  assert.equal(await rows.count(), PAGES.length + 1, "every tab is listed");
  await page.screenshot({ path: `${OUT}/phone-390-flat.png` });

  // Long-press a row, then drag it right by `dx` onto the lower half of `onto`.
  const touchDrag = async (from, onto, dx) => {
    const a = await rows.nth(from).boundingBox();
    const b = await rows.nth(onto).boundingBox();
    const x0 = a.x + 60, y0 = a.y + a.height / 2;
    const fire = (type, x, y) => page.evaluate(([type, x, y]) => {
      const target = type === "pointerdown" ? document.elementFromPoint(x, y) : document;
      target.dispatchEvent(new PointerEvent(type, {
        bubbles: true, cancelable: true, pointerType: "touch", pointerId: 7, isPrimary: true,
        button: type === "pointermove" ? -1 : 0, buttons: type === "pointerup" ? 0 : 1, clientX: x, clientY: y,
      }));
    }, [type, x, y]);
    await fire("pointerdown", x0, y0);
    await sleep(700); // past LONG_PRESS_DELAY
    const y1 = b.y + b.height * 0.75;
    for (let s = 1; s <= 5; s++) await fire("pointermove", x0 + (dx * s) / 5, y0 + ((y1 - y0) * s) / 5);
    await fire("pointerup", x0 + dx, y1);
    await sleep(150);
  };
  // Row 2 (org-sink) under row 1 (kitchen-sink), then row 3 under row 2.
  await touchDrag(2, 2, 24);
  await touchDrag(3, 3, 40);
  const depths = await rows.evaluateAll((els) => els.map((el) => Number(el.dataset.tabDepth ?? 0)));
  assert.deepEqual(depths, [0, 0, 1, 2], `long-press drag nests (depths ${depths})`);
  await page.screenshot({ path: `${OUT}/phone-390-nested.png` });

  // A held finger that moves at once is a scroll, not a drag: nothing moves.
  const before = await rows.allTextContents();
  await page.evaluate(() => {
    const row = document.querySelectorAll('[data-sidebar-section="tabs"] .nav-tab')[1];
    const r = row.getBoundingClientRect();
    const ev = (type, y) => new PointerEvent(type, { bubbles: true, cancelable: true, pointerType: "touch", pointerId: 8, isPrimary: true, button: 0, buttons: 1, clientX: r.x + 60, clientY: y });
    row.querySelector(".nav-tab-main").dispatchEvent(ev("pointerdown", r.y + 10));
    document.dispatchEvent(ev("pointermove", r.y + 80));
    document.dispatchEvent(ev("pointerup", r.y + 80));
  });
  await sleep(700);
  assert.deepEqual(await rows.allTextContents(), before, "a quick swipe does not reorder");

  await rows.nth(1).locator(".nav-tab-toggle").tap();
  await sleep(150);
  assert.equal(await rows.count(), 2, "collapsing hides the group's children in the list");
  await page.screenshot({ path: `${OUT}/phone-390-collapsed.png` });
  console.log(`phone-390: depths=${depths.join(",")} collapsed rows=${await rows.count()}`);
  await context.close();
} finally {
  await browser?.close();
  server.kill();
}
