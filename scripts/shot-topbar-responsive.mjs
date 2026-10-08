// Browser geometry and screenshot proof for GH #205.
// Usage: npm run build && node scripts/shot-topbar-responsive.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5225;
const OUT = path.resolve(process.env.TOPBAR_SHOT_DIR || "notes");
// GH #205 has real-fit tiers (master a006f1308). A phone (platform "android":
// no tab strip, no window controls) keeps calendar/journals/theme and the
// right-sidebar action direct down to a 345px content box. A desktop topbar
// carries a tab strip, and a frameless one three window buttons: those collapse
// optional actions at 460px so a tab pill stays readable (og H). Back/Forward
// stay inline until the 300px floor; the sidebar action moves to overflow only
// at the 250px last resort (or at 460px with window controls).
const CASES = [
  { name: "desktop-900", width: 900, sidebar: "open", menu: false, nav: 2, optional: 3, sidebarAction: 1, overflow: false },
  { name: "system-frame-collapse-440", width: 440, sidebar: "closed", menu: true, nav: 2, optional: 0, sidebarAction: 1, overflow: true,
    menuActions: ["calendar", "journals", "theme"], separator: false, minTabWidth: 100 },
  { name: "system-frame-nav-collapse-400", width: 400, sidebar: "closed", menu: true, nav: 0, optional: 0, sidebarAction: 1, overflow: true,
    menuActions: ["calendar", "journals", "theme", "back", "forward"], separator: true, minTabWidth: 100 },
  { name: "custom-frame-collapse-440", width: 440, sidebar: "closed", fakeWindowControls: true, menu: true, nav: 2, optional: 0, sidebarAction: 0, overflow: true,
    menuActions: ["calendar", "journals", "theme", "right-sidebar"], separator: false },
  { name: "phone-actions-inline-400", width: 400, platform: "android", sidebar: "closed", menu: false, nav: 2, optional: 3, sidebarAction: 1, overflow: false },
  { name: "phone-actions-inline-390", width: 390, platform: "android", sidebar: "closed", menu: false, nav: 2, optional: 3, sidebarAction: 1, overflow: false },
  { name: "phone-optional-collapse-360", width: 360, platform: "android", sidebar: "closed", menu: true, nav: 2, optional: 0, sidebarAction: 1, overflow: true,
    menuActions: ["calendar", "journals", "theme"], separator: false },
  { name: "nav-collapse-280", width: 280, sidebar: "closed", menu: true, nav: 0, optional: 0, sidebarAction: 1, overflow: true,
    menuActions: ["calendar", "journals", "theme", "back", "forward"], separator: true },
  { name: "last-resort-240", width: 240, sidebar: "closed", menu: true, nav: 0, optional: 0, sidebarAction: 0, overflow: true,
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
  // Two toolbar controls drawn over each other (tab-strip contents excluded:
  // the strip scrolls and clips its own pills).
  const boxes = visibleButtons.filter((button) => !button.closest(".tab-strip-scroll"))
    .map((button) => ({ label: button.getAttribute("aria-label") || button.title || button.textContent?.trim(), rect: button.getBoundingClientRect() }));
  const overlapping = boxes.flatMap((a, i) => boxes.slice(i + 1)
    .filter((b) => Math.min(a.rect.right, b.rect.right) - Math.max(a.rect.left, b.rect.left) > 1
      && Math.min(a.rect.bottom, b.rect.bottom) - Math.max(a.rect.top, b.rect.top) > 1)
    .map((b) => `${a.label} / ${b.label}`));
  const strip = topbar.querySelector(".tab-strip-scroll")?.getBoundingClientRect();
  const pill = topbar.querySelector(".tab.active")?.getBoundingClientRect();
  const activeTabVisibleWidth = strip && pill ? Math.max(0, Math.min(strip.right, pill.right) - Math.max(strip.left, pill.left)) : null;
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
    overlapping,
    activeTabVisibleWidth,
    // Probe geometry, not the element's own display: a child of a
    // display:none parent still reports its specified display.
    overflowVisible: overflow ? visible(overflow) : false,
    compactFallback: Boolean(topbar.querySelector('[data-workspace-switcher-compact="true"]')),
    fullSwitcherInTopbar: Boolean(topbar.querySelector('[data-workspace-switcher]:not([data-workspace-switcher-compact="true"])')),
    fullSwitcherInSidebar: Boolean(document.querySelector("[data-workspace-switcher-sidebar] [data-workspace-switcher]")),
    visibleNavigation: [...topbar.querySelectorAll(".topbar-navigation-action")].filter(visible).length,
    visibleOptional: [...topbar.querySelectorAll(".topbar-optional-action")].filter(visible).length,
    // By role, not class: the user-visible question is whether the direct
    // right-sidebar button is on the bar.
    visibleSidebarAction: [...topbar.querySelectorAll('button[title^="Toggle right sidebar"]')].filter(visible).length,
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
  // TOPBAR_CASES=name1,name2 runs a subset (for focused fail-before proof).
  const only = process.env.TOPBAR_CASES?.split(",").filter(Boolean);
  for (const testCase of only ? CASES.filter((c) => only.includes(c.name)) : CASES) {
    const page = await browser.newPage({ viewport: { width: testCase.width, height: 760 }, deviceScaleFactor: 1 });
    // makeSidebar's click toggle is pre-existing-broken at <=430px. Seed the
    // persisted state before the app module reads it instead of toggling live.
    if (testCase.sidebar === "closed") {
      await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
    }
    if (testCase.platform) {
      await page.addInitScript((platform) => { globalThis.__TINE_PLATFORM__ = platform; }, testCase.platform);
    }
    const target = new URL(baseUrl);
    target.searchParams.set("topbar205", testCase.name);
    await page.goto(target.href);
    await page.waitForSelector("header.topbar", { timeout: 8_000 });
    if (testCase.fakeWindowControls) {
      await page.evaluate(() => {
        const controls = document.createElement("div");
        controls.className = "win-controls";
        controls.setAttribute("data-test-window-controls", "true");
        document.querySelector(".topbar-right")?.append(controls);
      });
    }
    await sleep(180);
    const before = await page.evaluate(measureTopbar);
    if (before.clipped.length) throw new Error(`${testCase.name}: clipped toolbar buttons: ${before.clipped.join(", ")}`);
    if (before.overlapping.length) throw new Error(`${testCase.name}: overlapping toolbar buttons: ${before.overlapping.join(", ")}`);
    if (testCase.minTabWidth && !(before.activeTabVisibleWidth >= testCase.minTabWidth)) {
      throw new Error(`${testCase.name}: tab pill squeezed to ${before.activeTabVisibleWidth}px`);
    }
    if (before.visibleNavigation !== testCase.nav || before.visibleOptional !== testCase.optional
      || before.visibleSidebarAction !== testCase.sidebarAction || before.overflowVisible !== testCase.overflow) {
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
