// Open Settings → Backups (with the ?conflicts gate so the mock surfaces a
// sync-conflict copy), screenshot the panel, then open the merge modal and
// screenshot it. Verifies the sync-conflict reconcile + block-merge UI.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5209;
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
  const page = await browser.newPage({ viewport: { width: 900, height: 1180 } });
  await page.goto(`http://localhost:${PORT}/?conflicts`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.locator('button.icon-btn[title^="Settings"]').first().click();
  await page.waitForSelector(".settings-modal", { timeout: 3000 });
  await page.locator(".settings-nav-item", { hasText: "Journals" }).click();
  await sleep(300);
  // Scroll the sync-conflict panel into view and screenshot it.
  await page.locator(".sync-conflict-row").first().scrollIntoViewIfNeeded();
  await sleep(150);
  await page.locator(".settings-modal").screenshot({ path: "screenshots/syncconflicts-panel.png" });
  // Open the merge modal.
  await page.locator(".settings-btn", { hasText: "Review & merge" }).first().click();
  await page.waitForSelector(".sync-merge-modal", { timeout: 3000 });
  await sleep(300);
  await page.locator(".sync-merge-modal").screenshot({ path: "screenshots/syncconflicts-merge.png" });
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
