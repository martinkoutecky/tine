// Screenshot Chunk 2 UI: grid edge-grow affordances (hover) + board group-by toolbar.
// Usage: npm run build && node scripts/shot-chunk2.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5197;
const OUT_GRID = "/tmp/shot-chunk2-grid.png";
const OUT_BOARD = "/tmp/shot-chunk2-board.png";
const OUT_EMPTY = "/tmp/shot-chunk2-empty.png";

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
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
  await page.waitForSelector(".block-sheet-container > .sheet-scroll > .sheet-grid", { timeout: 5000 });
  await sleep(500);

  // Grid: hover the top-level grid to reveal the + edge affordances.
  const grid = page.locator(".block-sheet-container > .sheet-scroll > .sheet-grid").first();
  await grid.scrollIntoViewIfNeeded();
  await grid.hover({ position: { x: 30, y: 20 } });
  await sleep(300);
  const addCol = await page.locator(".sheet-grid-add-col").count();
  const addRow = await page.locator(".sheet-grid-add-row").count();
  const addColVisible = await page.locator(".sheet-grid-add-col.sheet-grid-add-visible").count();
  const gbox = await grid.boundingBox();
  await page.screenshot({
    path: OUT_GRID,
    clip: { x: Math.max(0, gbox.x - 8), y: Math.max(0, gbox.y - 8), width: gbox.width + 36, height: gbox.height + 36 },
  });

  // Board: the group-by toolbar is always visible.
  const boardWrap = page.locator(".sheet-board-wrap").first();
  await boardWrap.scrollIntoViewIfNeeded();
  await sleep(300);
  const toolbar = await page.locator(".sheet-board-toolbar .sheet-board-groupby").count();
  const groupVal = await page.locator(".sheet-board-groupby").first().inputValue().catch(() => "?");
  await boardWrap.screenshot({ path: OUT_BOARD }).catch(async () => { await page.screenshot({ path: OUT_BOARD, fullPage: true }); });

  console.log(JSON.stringify({ addCol, addRow, addColVisible, toolbar, groupVal, errors }, null, 2));
  await browser.close();
  server.kill("SIGKILL");
  process.exit(errors.length ? 1 : 0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
