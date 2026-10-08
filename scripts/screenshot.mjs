// Serve the built frontend (mock backend) and capture screenshots for visual
// review. Usage: node scripts/screenshot.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5191;
const OUT = "screenshots";
const NOTES_OUT = "subagent-tasks/notes";
mkdirSync(OUT, { recursive: true });
mkdirSync(NOTES_OUT, { recursive: true });

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], {
  stdio: "inherit",
});

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try {
      const r = await fetch(url);
      if (r.ok) return;
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
  const page = await browser.newPage({ viewport: { width: 1280, height: 860 }, deviceScaleFactor: 2 });

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 5000 });
  await sleep(400);
  await page.screenshot({ path: `${OUT}/journals-light.png` });

  // In-page find on normal pages: model-backed count + non-destructive highlight.
  await page.keyboard.press("Control+f");
  await page.waitForSelector(".inpage-find-bar", { timeout: 5000 });
  await page.keyboard.type("Tine");
  await sleep(350);
  await page.screenshot({ path: `${NOTES_OUT}/inpage-find-light.png` });
  await page.keyboard.press("Escape");
  await sleep(150);

  // Editing state: click a block to show the textarea editor.
  await page.locator(".block-content").nth(1).click();
  await sleep(250);
  await page.screenshot({ path: `${OUT}/editing-light.png` });

  // Autocomplete: type a [[ trigger and capture the popup.
  await page.keyboard.press("End");
  await page.keyboard.type(" [[lo");
  await sleep(300);
  await page.screenshot({ path: `${OUT}/autocomplete-light.png` });
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  // Quick switcher (Ctrl-K).
  await page.keyboard.press("Control+k");
  await sleep(200);
  await page.keyboard.type("log");
  await sleep(250);
  await page.screenshot({ path: `${OUT}/switcher-light.png` });
  await page.keyboard.press("Escape");
  await sleep(150);

  // Logbook badge + tooltip on the kitchen-sink task with a CLOCK drawer.
  await page.keyboard.press("Control+k");
  await sleep(150);
  await page.keyboard.type("kitchen-sink");
  await sleep(250);
  await page.keyboard.press("Enter");
  await page.waitForSelector(".clock-badge", { timeout: 5000 });
  const clockBlock = page.locator(".ls-block", { has: page.locator(".clock-badge") }).first();
  await clockBlock.scrollIntoViewIfNeeded();
  await page.locator(".clock-badge").first().hover();
  await page.waitForSelector(".clock-tooltip", { state: "visible", timeout: 5000 });
  await sleep(250);
  const blockBox = await clockBlock.boundingBox();
  const tooltipBox = await page.locator(".clock-tooltip").first().boundingBox();
  if (blockBox && tooltipBox) {
    const x = Math.max(0, Math.min(blockBox.x, tooltipBox.x) - 12);
    const y = Math.max(0, Math.min(blockBox.y, tooltipBox.y) - 12);
    const right = Math.min(1280, Math.max(blockBox.x + blockBox.width, tooltipBox.x + tooltipBox.width) + 12);
    const bottom = Math.min(860, Math.max(blockBox.y + blockBox.height, tooltipBox.y + tooltipBox.height) + 12);
    await page.screenshot({
      path: `${NOTES_OUT}/logbook-badge-tooltip.png`,
      clip: { x, y, width: right - x, height: bottom - y },
    });
  }

  // A named page (shows page properties + Linked References).
  await page.keyboard.press("Control+k");
  await sleep(150);
  await page.keyboard.type("Tine");
  await sleep(250);
  await page.keyboard.press("Enter");
  await page.waitForSelector(".pdf-link", { timeout: 5000 });
  await sleep(400);
  await page.screenshot({ path: `${OUT}/page-light.png` });

  // PDF viewer: open the PDF asset link on the page.
  await page.locator(".pdf-link").first().click();
  await page.waitForSelector(".pdf-page canvas", { timeout: 8000 }).catch(() => {});
  await sleep(1200);
  await page.screenshot({ path: `${OUT}/pdf-light.png` });

  // Try selecting a line of text to trigger the highlight color menu.
  const span = page.locator(".pdf-page .textLayer span").first();
  if (await span.count()) {
    await span.click({ clickCount: 3 });
    await sleep(300);
    await page.screenshot({ path: `${OUT}/pdf-select-light.png` });
    const swatch = page.locator(".pdf-color-swatch").first();
    if (await swatch.count()) {
      await swatch.click();
      await sleep(300);
      await page.screenshot({ path: `${OUT}/pdf-highlight-light.png` });

      // Open the highlights/notes page from the PDF toolbar.
      await page.locator(".pdf-notes-btn").click();
      await sleep(400);
      await page.screenshot({ path: `${OUT}/pdf-notes-light.png` });
    }
  }

  // Tabs: middle-click a page and Journals to open new tabs, pin one.
  await page.locator(".nav-item").first().click();
  await sleep(200);
  await page.locator(".nav-page").first().click({ button: "middle" });
  await sleep(200);
  await page.locator(".tab").nth(1).dblclick(); // pin it
  await sleep(200);
  await page.screenshot({ path: `${OUT}/tabs-light.png` });

  // Dark theme on the journals feed.
  const closePdf = page.locator('[title="Close PDF"]');
  if (await closePdf.count()) {
    await closePdf.click();
    await sleep(150);
  }
  await page.locator(".tab").filter({ hasText: "Journals" }).first().click();
  await sleep(150);
  await page.keyboard.press("Escape");
  await sleep(100);
  await page.click('[title^="Toggle theme"]');
  await sleep(300);
  await page.screenshot({ path: `${OUT}/journals-dark.png` });
  await page.keyboard.press("Control+f");
  await page.waitForSelector(".inpage-find-bar", { timeout: 5000 });
  await page.keyboard.type("Tine");
  await sleep(350);
  await page.screenshot({ path: `${NOTES_OUT}/inpage-find-dark.png` });

  await browser.close();
  console.log("screenshots written to", OUT);
} finally {
  server.kill("SIGTERM");
}
