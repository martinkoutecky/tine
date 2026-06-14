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

  // Dark theme on the journals feed.
  await page.locator(".nav-item").first().click();
  await sleep(200);
  await page.click(".icon-btn");
  await sleep(300);
  await page.screenshot({ path: `${OUT}/journals-dark.png` });

  await browser.close();
  console.log("screenshots written to", OUT);
} finally {
  server.kill("SIGTERM");
}
