import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
const PORT = 5196;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) { try { if ((await fetch(url)).ok) return; } catch {} await sleep(250); }
  throw new Error("server did not start");
}
try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 760, height: 640 } });
  const errs = [];
  page.on("console", (m) => m.type() === "error" && errs.push(m.text()));
  page.on("pageerror", (e) => errs.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 3000 });
  await page.locator(".switcher-input").fill("journal");
  await sleep(500);
  await page.screenshot({ path: "screenshots/switcher.png" });
  console.log(errs.length ? "ERRORS:\n" + errs.join("\n") : "no console errors");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) { console.error(String(e)); server.kill("SIGKILL"); process.exit(1); }
