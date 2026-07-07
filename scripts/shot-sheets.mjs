// Render the Sheets phase-1 mock page and screenshot it.
// Usage: npm run build && node scripts/shot-sheets.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5196;
const OUT = "/tmp/shot-sheets.png";
const OUT_SEL = "/tmp/shot-sheets-sel.png";
const OUT_RANGE = "/tmp/shot-sheets-range.png";
const OUT_SEAM = "/tmp/shot-sheets-seam.png";
const OUT_TABLE = "/tmp/shot-sheets-table.png";
const OUT_BOARD = "/tmp/shot-sheets-board.png";

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], {
  stdio: "ignore",
});

async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // not up yet
    }
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({
    args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"],
  });
  const page = await browser.newPage({ viewport: { width: 1120, height: 820 }, deviceScaleFactor: 2 });
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 3000 });
  await page.locator(".switcher-input").fill("Sheets demo");
  await sleep(350);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".block-sheet-container > .sheet-grid", { timeout: 5000 });
  await page.waitForSelector(".sheet-cell .sheet-grid", { timeout: 5000 });
  await sleep(500);
  await page.screenshot({ path: OUT, fullPage: true });

  const hole = page.locator(".block-sheet-container > .sheet-grid > .sheet-hole").first();
  if (await hole.count()) {
    await hole.click();
  } else {
    const firstCell = page.locator(".block-sheet-container > .sheet-grid > .sheet-cell").first();
    const box = await firstCell.boundingBox();
    await firstCell.click({ position: { x: Math.max(1, (box?.width ?? 12) - 4), y: Math.max(1, (box?.height ?? 12) - 4) } });
  }
  await page.waitForSelector(".block-sheet-container > .sheet-grid > .sheet-cell-selected", { timeout: 3000 });
  await sleep(250);
  await page.screenshot({ path: OUT_SEL, fullPage: true });

  const rangeDir = await page
    .locator(".block-sheet-container > .sheet-grid > .sheet-cell-selected")
    .first()
    .evaluate((el) => (Number(el.getAttribute("data-col") || "0") > 0 ? "ArrowLeft" : "ArrowRight"));
  await page.keyboard.down("Shift");
  await page.keyboard.press(rangeDir);
  await page.keyboard.up("Shift");
  await page.waitForSelector(".block-sheet-container > .sheet-grid > .sheet-cell-in-range", { timeout: 3000 });
  await sleep(250);
  await page.screenshot({ path: OUT_RANGE, fullPage: true });

  await page.keyboard.press("Escape");
  await page.waitForSelector(".block-sheet-container > .sheet-grid > .sheet-cell-selected", { timeout: 3000 });

  await page.keyboard.press("ArrowRight");
  await page.waitForSelector(".block-sheet-container > .sheet-grid > .sheet-seam-selected", { timeout: 3000 });
  await sleep(250);
  await page.screenshot({ path: OUT_SEAM, fullPage: true });

  await page.waitForSelector(".sheet-table", { timeout: 3000 });
  await page.locator(".sheet-table").first().scrollIntoViewIfNeeded();
  await sleep(250);
  await page.screenshot({ path: OUT_TABLE, fullPage: true });

  await page.waitForSelector(".sheet-board", { timeout: 3000 });
  await page.locator(".sheet-board").first().scrollIntoViewIfNeeded();
  await sleep(250);
  await page.screenshot({ path: OUT_BOARD, fullPage: true });

  console.log(
    errors.length
      ? "ERRORS:\n" + errors.join("\n")
      : `wrote ${OUT}, ${OUT_SEL}, ${OUT_RANGE}, ${OUT_SEAM}, ${OUT_TABLE}, and ${OUT_BOARD}`
  );
  await browser.close();
  server.kill("SIGKILL");
  process.exit(errors.length ? 1 : 0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
