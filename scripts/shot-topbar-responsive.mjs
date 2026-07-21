// Browser geometry and screenshot proof for GH #205.
// Usage: npm run build && node scripts/shot-topbar-responsive.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5225;
const OUT = path.resolve(process.env.TOPBAR_SHOT_DIR || "notes");
// GH #205 has two container-query tiers: optional actions move into "…" at
// 460px, while Back/Forward stay inline until the 300px floor. Exercise both
// tiers and a roomy desktop neighbor.
const CASES = [
  { name: "desktop-900", width: 900, sidebar: "open", menu: false, nav: 2, optional: 4, overflow: false },
  { name: "optional-collapse-440", width: 440, sidebar: "closed", menu: true, nav: 2, optional: 0, overflow: true,
    menuActions: ["calendar", "journals", "theme", "right-sidebar"], separator: false },
  { name: "phone-nav-inline-390", width: 390, sidebar: "closed", menu: true, nav: 2, optional: 0, overflow: true,
    menuActions: ["calendar", "journals", "theme", "right-sidebar"], separator: false },
  { name: "nav-collapse-280", width: 280, sidebar: "closed", menu: true, nav: 0, optional: 0, overflow: true,
    menuActions: ["calendar", "journals", "theme", "right-sidebar", "back", "forward"], separator: true },
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
  const visible = (element) => {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const overflow = document.querySelector("[data-topbar-overflow-trigger]");
  return {
    topbar: { left: bar.left, right: bar.right, width: bar.width },
    buttons: visibleButtons.map((button) => button.getAttribute("aria-label") || button.title || button.textContent?.trim()),
    clipped,
    // Probe geometry, not the element's own display: a child of a
    // display:none parent still reports its specified display.
    overflowVisible: overflow ? visible(overflow) : false,
    compactFallback: Boolean(topbar.querySelector('[data-workspace-switcher-compact="true"]')),
    fullSwitcherInTopbar: Boolean(topbar.querySelector('[data-workspace-switcher]:not([data-workspace-switcher-compact="true"])')),
    fullSwitcherInSidebar: Boolean(document.querySelector("[data-workspace-switcher-sidebar] [data-workspace-switcher]")),
    visibleNavigation: [...topbar.querySelectorAll(".topbar-navigation-action")].filter(visible).length,
    visibleOptional: [...topbar.querySelectorAll(".topbar-optional-action")].filter(visible).length,
    visibleOverflowActions: [...topbar.querySelectorAll("[data-topbar-overflow-action]")]
      .filter(visible)
      .map((element) => element.getAttribute("data-topbar-overflow-action")),
    overflowSeparatorVisible: [...topbar.querySelectorAll(".topbar-overflow-sep")].some(visible),
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
    // makeSidebar's click toggle is pre-existing-broken at <=430px. Seed the
    // persisted state before the app module reads it instead of toggling live.
    if (testCase.sidebar === "closed") {
      await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
    }
    const target = new URL(baseUrl);
    target.searchParams.set("topbar205", testCase.name);
    await page.goto(target.href);
    await page.waitForSelector("header.topbar", { timeout: 8_000 });
    await sleep(180);
    const before = await page.evaluate(measureTopbar);
    if (before.clipped.length) throw new Error(`${testCase.name}: clipped toolbar buttons: ${before.clipped.join(", ")}`);
    if (before.visibleNavigation !== testCase.nav || before.visibleOptional !== testCase.optional || before.overflowVisible !== testCase.overflow) {
      throw new Error(`${testCase.name}: wrong topbar tier: ${JSON.stringify(before)}`);
    }
    if (testCase.sidebar === "closed" && (!before.compactFallback || before.fullSwitcherInTopbar)) {
      throw new Error(`${testCase.name}: closed-sidebar fallback state is wrong: ${JSON.stringify(before)}`);
    }
    if (testCase.sidebar === "open" && (!before.fullSwitcherInSidebar || before.compactFallback || before.fullSwitcherInTopbar)) {
      throw new Error(`${testCase.name}: sidebar workspace placement is wrong: ${JSON.stringify(before)}`);
    }
    let opened;
    if (testCase.menu) {
      await page.locator("[data-topbar-overflow-trigger]").click();
      opened = await page.evaluate(measureTopbar);
      if (JSON.stringify(opened.visibleOverflowActions) !== JSON.stringify(testCase.menuActions)
        || opened.overflowSeparatorVisible !== testCase.separator) {
        throw new Error(`${testCase.name}: wrong overflow contents: ${JSON.stringify(opened)}`);
      }
    }
    await page.screenshot({ path: path.join(OUT, `205-topbar-${testCase.name}.png`) });
    console.log(JSON.stringify({ case: testCase.name, ...before, opened }));
    await page.close();
  }
} finally {
  await browser?.close();
  server?.kill("SIGTERM");
}
