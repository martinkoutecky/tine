// Verify + screenshot the query-builder Sort popover redesign: a grid of
// one-click presets (Newest first / Priority / Page / Deadline / …) over a
// free-text property fallback — so the common cases need no typing. Headless
// Chromium over the mock backend (the "Jun 14th, 2026" journal has a pure
// {{query}} block whose builder bar shows the "+ sort" control).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5216;
const OUT = "screenshots";
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1200, height: 1300 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  const sortBtn = page.locator(".qb-sort").first();
  await sortBtn.scrollIntoViewIfNeeded();
  await sortBtn.click();
  await page.waitForSelector(".qb-sort-picker", { timeout: 4000 });
  await sleep(250);

  const presets = await page.locator(".qb-sort-preset").allInnerTexts();
  console.log("presets:", presets.join(" | "));

  // Screenshot the open popover. Clip generously around the picker element itself.
  const bar = page.locator(".qb-bar").first();
  const box = await bar.boundingBox();
  const pick = await page.locator(".qb-sort-picker").boundingBox();
  console.log("bar box:", JSON.stringify(box), "| picker box:", JSON.stringify(pick));
  await page.screenshot({ path: `${OUT}/sort-full.png` });
  console.log(`wrote ${OUT}/sort-full.png (full viewport)`);
  if (pick) {
    await page.screenshot({
      path: `${OUT}/sort-popover.png`,
      clip: {
        x: Math.max(0, pick.x - 60),
        y: Math.max(0, (box?.y ?? pick.y) - 12),
        width: Math.min(1200 - Math.max(0, pick.x - 60), pick.width + 120),
        height: (pick.y - (box?.y ?? pick.y)) + pick.height + 24,
      },
    });
    console.log(`wrote ${OUT}/sort-popover.png (picker ${Math.round(pick.width)}x${Math.round(pick.height)})`);
  }

  // Apply "Newest first" and confirm the chip reflects it + popover closed.
  await page.locator(".qb-sort-preset", { hasText: "Newest first" }).click();
  await sleep(300);
  const chip = await page.locator(".qb-chip", { hasText: "sort:" }).allInnerTexts().catch(() => []);
  const stillOpen = await page.locator(".qb-sort-picker").count();
  console.log("after apply — sort chip(s):", chip.join(" | ") || "(none found)", "| popover open:", stillOpen);

  await page.screenshot({ path: `${OUT}/sort-applied.png`, clip: { x: 0, y: Math.max(0, (box?.y ?? 100) - 40), width: 1000, height: 200 } });
  console.log(`wrote ${OUT}/sort-applied.png`);

  await browser.close();
} catch (e) {
  console.log("ERROR:", String(e).split("\n").slice(0, 3).join(" | "));
  process.exitCode = 2;
} finally {
  server.kill("SIGTERM");
}
