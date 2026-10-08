// 0.5.0 landing-site screenshots (website/img/): the two headline features the
// site was missing — Sheets and Split view. Headless Chromium over the mock
// backend, same rig as shot-website.mjs.
//
//   web-sheets.png — the Sheets demo page (grid + field table + board)
//   web-split.png  — a two-pane split: outline on the left, the sheet on the right
//
// Usage: source scripts/env.sh && npm run build && node scripts/shot-website-050.mjs
//        then copy the chosen files into website/img/ (see docs/SCREENSHOTS.md).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5197;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

async function openSheetsDemo(page) {
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 4000 });
  await page.locator(".switcher-input").fill("Sheets demo");
  await sleep(350);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".block-sheet-container .sheet-grid", { timeout: 6000 });
  await sleep(500);
}

async function dismissToasts(page) {
  for (const c of await page.locator(".toast-close").all()) await c.click().catch(() => {});
}

const errors = [];
try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });

  // --- 1. Sheets demo (grid + table + board) -----------------------------
  {
    const ctx = await browser.newContext({ viewport: { width: 1180, height: 860 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => errors.push(String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".ls-block", { timeout: 8000 });
    await dismissToasts(page);
    await openSheetsDemo(page);
    await dismissToasts(page);
    await page.screenshot({ path: `${OUT}/web-sheets.png` });
    console.log("OK    web-sheets");
    await ctx.close();
  }

  // --- 2. Split view + pane-select: A | (B/C), with B highlighted --------
  {
    const ctx = await browser.newContext({ viewport: { width: 1360, height: 840 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => errors.push(String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    await dismissToasts(page);
    await openSheetsDemo(page); // A (left) is a rich, tall pane
    await dismissToasts(page);

    const rects = () => page.evaluate(() => [...document.querySelectorAll(".pane-leaf")].map((el) => {
      const r = el.getBoundingClientRect();
      return { id: el.getAttribute("data-pane-id"), x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
    }));
    const sel = () => page.evaluate(() => {
      const el = document.querySelector(".pane-leaf.pane-selected");
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { id: el.getAttribute("data-pane-id"), x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
    });
    const enterPS = async () => { for (let i = 0; i < 3; i++) { await page.keyboard.press("Escape"); await sleep(140); } };

    // Split right → [A | pane-1] (focus pane-1), then split pane-1 DOWN via the
    // real UX (pane-select → its bottom edge → Enter) since the split-down chord
    // is unreachable by keyboard (Shift+\ → "|").
    await page.keyboard.press("Control+Alt+Backslash");
    await page.waitForSelector(".pane-leaf", { timeout: 4000 });
    await sleep(300);
    await enterPS();
    await page.keyboard.press("ArrowDown"); await sleep(180); // pane-1's bottom edge
    await page.keyboard.press("Enter"); await sleep(400);      // split → A | (B/C)

    // Select the top-right pane (B) so its tint shows.
    await enterPS();
    const all = await rects();
    const cx = Math.max(...all.map((r) => r.x + r.w / 2));
    const B = all.filter((r) => r.x + r.w / 2 > cx - 30).sort((a, b) => a.y - b.y)[0];
    for (let step = 0; step < 5; step++) {
      const s = await sel();
      if (s && s.id === B.id) break;
      const k = !s ? "ArrowUp" : s.y > B.y + 20 ? "ArrowUp" : s.x + s.w / 2 < B.x + B.w / 2 - 20 ? "ArrowRight" : "ArrowUp";
      await page.keyboard.press(k); await sleep(180);
    }
    await dismissToasts(page);
    await page.screenshot({ path: `${OUT}/web-split.png` });
    console.log("OK    web-split");
    await ctx.close();
  }

  await browser.close();
  console.log(errors.length ? "ERRORS:\n" + errors.join("\n") : "done");
} finally {
  server.kill("SIGTERM");
}
process.exit(errors.length ? 1 : 0);
