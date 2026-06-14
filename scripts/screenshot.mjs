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
  await sleep(300);
  await page.screenshot({ path: `${OUT}/journals-light.png` });

  // A named page.
  await page.click("text=logseq-claude >> nth=0").catch(() => {});
  await sleep(300);
  await page.screenshot({ path: `${OUT}/page-light.png` });

  // Dark theme.
  await page.click(".icon-btn");
  await sleep(300);
  await page.screenshot({ path: `${OUT}/journals-dark.png` });

  await browser.close();
  console.log("screenshots written to", OUT);
} finally {
  server.kill("SIGTERM");
}
