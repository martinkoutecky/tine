// Serve the built frontend (mock backend) and capture screenshots for visual
// review. Usage: node scripts/screenshot.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5191;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });

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

  // A named page (shows page properties + Linked References).
  await page.locator(".nav-page").first().click();
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
  await page.locator(".tab").first().click();
  await sleep(150);
  await page.click('[title^="Toggle theme"]');
  await sleep(300);
  await page.screenshot({ path: `${OUT}/journals-dark.png` });

  await browser.close();
  console.log("screenshots written to", OUT);
} finally {
  server.kill("SIGTERM");
}
