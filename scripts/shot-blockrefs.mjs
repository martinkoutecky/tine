// Verify the per-block reference-count badge + referrers panel on the kitchen-sink
// target block (id 64b9c0e2…, referenced by a bare + a labeled ref → count 2).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5196;
const TARGET = "64b9c0e2-0000-0000-0000-000000000000";
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
  const page = await browser.newPage({ viewport: { width: 820, height: 1400 } });
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 3000 });
  await page.locator(".switcher-input").fill("kitchen");
  await sleep(400);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".ls-block", { timeout: 3000 });
  await sleep(500);

  const block = page.locator(`.ls-block[data-block-id="${TARGET}"]`);
  await block.scrollIntoViewIfNeeded();
  await sleep(200);
  const badge = block.locator(".block-refs-count").first();
  const badgeCount = await badge.count();
  const badgeText = badgeCount ? (await badge.innerText()) : "(no badge)";
  console.log("badge text:", badgeText);
  await block.screenshot({ path: "screenshots/blockref-badge.png" });

  // Click the badge → referrers panel expands below the block.
  if (badgeCount) {
    await badge.click();
    await page.waitForSelector(`.ls-block[data-block-id="${TARGET}"] .block-references`, { timeout: 3000 });
    await sleep(400);
    await block.screenshot({ path: "screenshots/blockref-panel.png" });
  }

  console.log(errors.length ? "ERRORS:\n" + errors.join("\n") : "no console errors");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
