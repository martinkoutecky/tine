// Visual regression for GH #88: a whole-block `{{embed ((uuid))}}` presents the
// referenced root as the one interactive root bullet, with a heavier descendant
// guide instead of an extra host bullet or enclosing box.
// Usage: npm run build && node scripts/shot-block-embed.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5206;
const OUT = "screenshots/block-embed-root.png";
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser;
let failed = false;

async function waitForServer(url) {
  for (let i = 0; i < 60; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 900, height: 600 } });
  page.setDefaultTimeout(5000);
  const errors = [];
  page.on("console", (message) => message.type() === "error" && errors.push(message.text()));
  page.on("pageerror", (error) => errors.push(String(error)));

  await page.goto(`http://localhost:${PORT}/?regressions`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Block embed regression");
  await page.locator(".switcher-row").first().click();

  const host = page.locator('.block-embed-host[data-block-id]');
  await host.getByText("Embedded grandchild", { exact: true }).waitFor({ timeout: 5000 });
  const bulletCount = await host.locator(".bullet-container").count();
  const hostControlCount = await host.locator(":scope > .block-main > .block-controls").count();
  const embeddedBulletCount = await host.locator(".embed-block .bullet-container").count();
  const rootToggle = host.locator(".embed-block .collapse-toggle.has-children").first();
  const rootToggleCount = await host.locator(".embed-block .collapse-toggle.has-children").count();
  const rootGuide = host.locator(
    ".embed-block > .live-ref-group > .ls-block > .block-children-container > .block-children",
  );
  const guideWidth = await rootGuide.evaluate((element) => getComputedStyle(element).borderLeftWidth);

  if (bulletCount !== 3) {
    // One root + child + grandchild. The removed host bullet would make four.
    throw new Error(`expected 3 outline bullets, got ${bulletCount}`);
  }
  if (hostControlCount !== 0) throw new Error(`macro host still has ${hostControlCount} control column(s)`);
  if (embeddedBulletCount !== 3) throw new Error(`embedded outline has ${embeddedBulletCount} bullets`);
  if (guideWidth !== "2px") throw new Error(`expected a 2px embedded-root guide, got ${guideWidth}`);
  if (rootToggleCount < 1) {
    throw new Error(`embedded live outline has no collapse control: ${await host.locator(".embed-block").innerHTML()}`);
  }
  const child = host.getByText("Embedded child", { exact: true });
  await rootToggle.click();
  await child.waitFor({ state: "detached", timeout: 3000 });
  await rootToggle.click();
  await host.getByText("Embedded grandchild", { exact: true }).waitFor({ timeout: 3000 });
  if (errors.length) throw new Error(`browser errors:\n${errors.join("\n")}`);

  await host.screenshot({ path: OUT });
  console.log(JSON.stringify({ out: OUT, bulletCount, hostControlCount, embeddedBulletCount, guideWidth }));
} catch (error) {
  console.error(String(error));
  failed = true;
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
if (failed) process.exit(1);
