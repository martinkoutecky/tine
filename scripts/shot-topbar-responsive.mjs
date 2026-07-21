// Browser geometry and screenshot proof for GH #205.
// Usage: npm run build && node scripts/shot-topbar-responsive.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5225;
const OUT = path.resolve(process.env.TOPBAR_SHOT_DIR || "notes");
// The collapse breakpoint is @container topbar (max-width: 460px). The fixed
// clusters (left icons + compact switcher + right icons) only need ~364px, so a
// mid-width window like 560px has plenty of room and must NOT collapse into the
// "…" menu (GH #205 follow-up: the old 760px threshold over-collapsed).
const CASES = [
  { name: "narrow", width: 360, sidebar: "closed", menu: true },
  { name: "collapse-edge", width: 430, sidebar: "closed", menu: true },
  { name: "mid", width: 560, sidebar: "closed", menu: false },
  { name: "desktop", width: 1280, sidebar: "open", menu: false },
];

mkdirSync(OUT, { recursive: true });
const configArgs = process.env.TINE_VITE_CONFIG ? ["--config", process.env.TINE_VITE_CONFIG] : [];
const baseUrl = process.env.TINE_SHOT_URL || `http://127.0.0.1:${PORT}/`;
const server = process.env.TINE_SHOT_URL
  ? undefined
  : spawn("npx", ["vite", "preview", ...configArgs, "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer() {
  if (!server) return;
  for (let i = 0; i < 60; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/`)).ok) return; } catch {}
    await sleep(200);
  }
  throw new Error("preview server did not start");
}

async function makeSidebar(page, wanted) {
  const compact = page.locator('[data-workspace-switcher-compact="true"]');
  const full = page.locator("[data-workspace-switcher-sidebar] [data-workspace-switcher]");
  const isClosed = async () => (await compact.count()) > 0 && (await full.count()) === 0;
  if ((wanted === "closed") !== await isClosed()) {
    await page.locator('button[title^="Toggle sidebar"]').click();
    await page.waitForFunction((closed) => {
      const compactSwitcher = document.querySelector('[data-workspace-switcher-compact="true"]');
      const sidebarSwitcher = document.querySelector("[data-workspace-switcher-sidebar] [data-workspace-switcher]");
      return closed ? Boolean(compactSwitcher) && !sidebarSwitcher : Boolean(sidebarSwitcher) && !compactSwitcher;
    }, wanted === "closed");
  }
}

function measureTopbar() {
  const topbar = document.querySelector("header.topbar");
  if (!topbar) throw new Error("topbar missing");
  const bar = topbar.getBoundingClientRect();
  const visibleButtons = [...topbar.querySelectorAll("button")]
    .filter((button) => {
      const style = getComputedStyle(button);
      const rect = button.getBoundingClientRect();
      return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
    });
  const clipped = visibleButtons
    .map((button) => ({ label: button.getAttribute("aria-label") || button.title || button.textContent?.trim(), rect: button.getBoundingClientRect() }))
    .filter(({ rect }) => rect.left < bar.left - 1 || rect.right > bar.right + 1)
    .map(({ label }) => label);
  const overflow = document.querySelector<HTMLElement>("[data-topbar-overflow-trigger]");
  return {
    topbar: { left: bar.left, right: bar.right, width: bar.width },
    buttons: visibleButtons.map((button) => button.getAttribute("aria-label") || button.title || button.textContent?.trim()),
    clipped,
    overflowVisible: overflow ? getComputedStyle(overflow).display !== "none" : false,
    compactFallback: Boolean(topbar.querySelector('[data-workspace-switcher-compact="true"]')),
    fullSwitcherInTopbar: Boolean(topbar.querySelector('[data-workspace-switcher]:not([data-workspace-switcher-compact="true"])')),
    fullSwitcherInSidebar: Boolean(document.querySelector("[data-workspace-switcher-sidebar] [data-workspace-switcher]")),
  };
}

let browser;
try {
  await waitForServer();
  browser = await chromium.launch({
    chromiumSandbox: false,
    args: [
      "--no-sandbox",
      "--disable-setuid-sandbox",
      "--disable-gpu",
      "--disable-dev-shm-usage",
      ...(process.env.TINE_SHOT_SINGLE_PROCESS ? ["--single-process", "--no-zygote"] : []),
    ],
  });
  for (const testCase of CASES) {
    const page = await browser.newPage({ viewport: { width: testCase.width, height: 760 }, deviceScaleFactor: 1 });
    const target = new URL(baseUrl);
    target.searchParams.set("topbar205", testCase.name);
    await page.goto(target.href);
    await page.waitForSelector("header.topbar", { timeout: 8_000 });
    await makeSidebar(page, testCase.sidebar);
    await sleep(180);
    const before = await page.evaluate(measureTopbar);
    if (before.clipped.length) throw new Error(`${testCase.name}: clipped toolbar buttons: ${before.clipped.join(", ")}`);
    if (testCase.menu && !before.overflowVisible) throw new Error(`${testCase.name}: overflow trigger is not visible`);
    if (!testCase.menu && before.overflowVisible) throw new Error(`${testCase.name}: overflow trigger is visible at desktop width`);
    if (testCase.sidebar === "closed" && (!before.compactFallback || before.fullSwitcherInTopbar)) {
      throw new Error(`${testCase.name}: closed-sidebar fallback state is wrong: ${JSON.stringify(before)}`);
    }
    if (testCase.sidebar === "open" && (!before.fullSwitcherInSidebar || before.compactFallback || before.fullSwitcherInTopbar)) {
      throw new Error(`${testCase.name}: sidebar workspace placement is wrong: ${JSON.stringify(before)}`);
    }
    if (testCase.menu) await page.locator("[data-topbar-overflow-trigger]").click();
    await page.screenshot({ path: path.join(OUT, `205-topbar-${testCase.name}.png`) });
    console.log(JSON.stringify({ case: testCase.name, ...before }));
    await page.close();
  }
} finally {
  await browser?.close();
  server?.kill("SIGTERM");
}
