// Open Settings → Backups tab → scan for orphaned assets, and screenshot the panel.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5197;
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
  const page = await browser.newPage({ viewport: { width: 900, height: 800 } });
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.locator('button.icon-btn[title^="Settings"]').first().click();
  await page.waitForSelector(".settings-modal", { timeout: 3000 });
  await page.locator(".settings-nav-item", { hasText: "Backups" }).click();
  await sleep(200);
  await page.locator(".settings-btn", { hasText: "Scan for orphans" }).click();
  await sleep(500);
  await page.locator(".settings-modal").screenshot({ path: "screenshots/assets-panel.png" });
  console.log(errors.length ? "ERRORS:\n" + errors.join("\n") : "no console errors");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
