// Open a page context menu → "Export to PDF…" → screenshot the export-options dialog.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5210;
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
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 760 } });
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.locator(".page-title").first().click({ button: "right" });
  await page.waitForSelector(".ctx-item", { timeout: 3000 });
  await page.locator(".ctx-item", { hasText: "Export to PDF" }).click();
  await page.waitForSelector(".pdf-export-modal", { timeout: 3000 });
  await sleep(200);
  await page.locator(".pdf-export-modal").screenshot({ path: "screenshots/pdfdialog.png" });
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
