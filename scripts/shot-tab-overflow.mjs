// Browser geometry and screenshot proof for GH #105 across widths and themes.
// Usage: npm run build && node scripts/shot-tab-overflow.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5216;
const OUT = "screenshots/tab-overflow";
const PAGES = [
  "Tine", "org-sink", "kitchen-sink", "Sheets demo", "Formula1",
  "Formula1/2026/08 Austrian Grand Prix", "Formula1/2026/09 Italian Grand Prix",
  "Formula1/2025/12 Abu Dhabi Grand Prix", "Preamble regression",
];
const CASES = [
  { name: "default-light-wide", width: 1280, mode: "light", theme: "" },
  { name: "default-dark-medium", width: 900, mode: "dark", theme: "" },
  { name: "nord-dark-narrow", width: 820, mode: "dark", theme: "nord" },
];

mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser;

async function waitForServer() {
  for (let i = 0; i < 50; i++) {
    try { if ((await fetch(`http://localhost:${PORT}/?regressions`)).ok) return; } catch {}
    await sleep(200);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer();
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  await page.goto(`http://localhost:${PORT}/?regressions`);
  await page.waitForSelector(".page-title", { timeout: 8000 });

  await page.keyboard.press("Control+k");
  const input = page.locator(".switcher-input");
  for (const name of PAGES) {
    const before = await page.locator(".tab-strip-scroll > .tab").count();
    await input.fill(name);
    const row = page.locator(".switcher-row")
      .filter({ has: page.locator(".switcher-kind", { hasText: /^page$/ }) })
      .filter({ hasText: name })
      .first();
    await row.waitFor({ timeout: 5000 });
    await row.evaluate((element) => {
      element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 1 }));
    });
    await page.waitForFunction((count) => document.querySelectorAll(".tab-strip-scroll > .tab").length > count, before);
  }
  await page.keyboard.press("Escape");

  // A synthetic HTMLElement.click() skips pointer capture. Use Playwright's
  // real Chromium pointer path so the close child must survive the draggable
  // tab card's pointerdown before its click can close the tab (GH #174).
  const beforePointerClose = await page.locator(".tab-strip-scroll > .tab").count();
  await page.locator(".tab-strip-scroll > .tab:last-child .tab-close").click();
  await page.waitForFunction(
    (count) => document.querySelectorAll(".tab-strip-scroll > .tab").length === count - 1,
    beforePointerClose,
  );

  for (const testCase of CASES) {
    await page.setViewportSize({ width: testCase.width, height: 720 });
    await page.evaluate(({ mode, theme }) => {
      document.documentElement.setAttribute("data-theme", mode);
      window.__tineApplyTheme?.(theme);
    }, testCase);
    await sleep(180);
    const trigger = page.locator("[data-tab-overview-trigger]");
    await trigger.waitFor({ timeout: 5000 });
    await trigger.click();
    const metrics = await page.evaluate(() => {
      const strip = document.querySelector(".tab-strip-scroll");
      const tabs = [...document.querySelectorAll(".tab-strip-scroll > .tab")];
      const rows = document.querySelectorAll("[data-tab-overview-row]");
      return {
        clientWidth: strip?.clientWidth ?? 0,
        scrollWidth: strip?.scrollWidth ?? 0,
        minTabWidth: Math.min(...tabs.map((tab) => tab.getBoundingClientRect().width)),
        tabs: tabs.length,
        rows: rows.length,
      };
    });
    if (!(metrics.scrollWidth > metrics.clientWidth)) throw new Error(`${testCase.name}: strip does not overflow`);
    if (metrics.minTabWidth < 110) throw new Error(`${testCase.name}: tab shrank to ${metrics.minTabWidth}px`);
    if (metrics.rows !== metrics.tabs) throw new Error(`${testCase.name}: ${metrics.rows} rows for ${metrics.tabs} tabs`);
    await page.screenshot({ path: `${OUT}/${testCase.name}.png` });
    await page.keyboard.press("Escape");
    console.log(JSON.stringify({ case: testCase.name, ...metrics }));
  }

  await page.locator("[data-tab-overview-trigger]").click();
  await page.locator("[data-tab-overview-row]").last().click();
  await sleep(100);
  const revealState = await page.evaluate(() => {
    const strip = document.querySelector(".tab-strip-scroll");
    const active = strip?.querySelector(".tab.active");
    if (!strip || !active) return { revealed: false };
    const s = strip.getBoundingClientRect();
    const a = active.getBoundingClientRect();
    return {
      revealed: a.left >= s.left - 1 && a.right <= s.right + 1,
      strip: { left: s.left, right: s.right, scrollLeft: strip.scrollLeft, clientWidth: strip.clientWidth },
      active: { left: a.left, right: a.right, offsetLeft: active.offsetLeft, width: active.offsetWidth },
    };
  });
  if (!revealState.revealed) throw new Error(`active tab was not revealed after overview selection: ${JSON.stringify(revealState)}`);
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
