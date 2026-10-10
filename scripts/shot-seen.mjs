// Self-verification shots for "changed since you last looked" (vision 9a,
// ADR 0073): open a page, Page actions → Mark page seen, edit three blocks, and
// screenshot the margin bars and the header at light/dark × desktop/phone.
// Usage: npm run build && node scripts/shot-seen.mjs   (needs scripts/env.sh for Chromium)
// Output: SHOT_DIR (default scratch/seen/).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5231;
const OUT = process.env.SHOT_DIR || "scratch/seen";
mkdirSync(OUT, { recursive: true });
const server = spawn("./node_modules/.bin/vite", ["preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url) {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(url)).ok) return; } catch { /* not up yet */ }
    await sleep(250);
  }
  throw new Error("server did not start");
}

const EDITED = ["Reads the same markdown graph", "Architecture", "Rust core owns parsing"];

async function run(browser, { theme, phone }) {
  const context = await browser.newContext({
    viewport: phone ? { width: 400, height: 860 } : { width: 1180, height: 760 },
    deviceScaleFactor: 2,
    colorScheme: theme,
  });
  // The theme preference lives in local storage (src/themePreference.ts).
  await context.addInitScript((t) => localStorage.setItem("logseq-claude.theme", t), theme);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.setDefaultTimeout(8000);
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block");
  await page.evaluate(() => document.querySelectorAll(".toast-close").forEach((b) => b.click()));
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Tine");
  await sleep(400);
  await page.keyboard.press("Enter"); // an exact title match leads the list
  await page.waitForSelector(`.ls-block:has-text("${EDITED[0]}")`);
  // A phone opens with the navigation drawer over the page; close it.
  if (phone && (await page.locator("[data-mobile-drawer-panel]").count())) {
    await page.locator('button[aria-label="Close navigation sidebar"]').click();
    await page.waitForFunction(() => document.querySelectorAll("[data-mobile-drawer-panel]").length === 0);
  }
  const html = await page.evaluate(() => document.documentElement.dataset.theme);
  if (html !== theme) throw new Error(`theme ${html} != ${theme}`);
  if (await page.locator(".seen-changed, [data-seen-header]").count()) throw new Error("untracked page shows seen chrome");

  await page.locator("[data-page-actions-trigger]").first().click();
  await page.locator('[role="menuitem"]', { hasText: "Mark page seen" }).click();
  await sleep(300);
  if (await page.locator(".seen-changed").count()) throw new Error("freshly marked page shows changes");

  for (const text of EDITED) {
    const content = page.locator(".ls-block > .block-main", { hasText: text }).first().locator(".block-content").first();
    await content.click({ position: { x: 4, y: 6 } });
    await page.keyboard.press("Control+End");
    await page.keyboard.type(" (edited)");
    await page.keyboard.press("Escape");
    await sleep(150);
  }
  await page.evaluate(() => document.activeElement?.blur?.());
  await page.waitForSelector("[data-seen-header]");
  const header = (await page.locator("[data-seen-header]").textContent())?.trim();
  const bars = await page.locator(".block-main.seen-changed").count();
  if (bars !== 3 || !header?.startsWith("3 changes since you last looked")) {
    throw new Error(`expected 3 bars and a 3-change header; got ${bars} bars, header "${header}"`);
  }
  await sleep(300);
  const name = `${theme}-${phone ? "phone" : "desktop"}.png`;
  await page.screenshot({ path: `${OUT}/${name}` });
  await context.close();
  if (errors.length) throw new Error(`page errors (${name}):\n${errors.join("\n")}`);
  return name;
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu"] });
  const shots = [];
  for (const theme of ["light", "dark"]) for (const phone of [false, true]) shots.push(await run(browser, { theme, phone }));
  await browser.close();
  server.kill("SIGKILL");
  console.log(`seen shots: ${shots.map((s) => `${OUT}/${s}`).join(" ")}`);
} catch (e) {
  server.kill("SIGKILL");
  console.error(e);
  process.exit(1);
}
