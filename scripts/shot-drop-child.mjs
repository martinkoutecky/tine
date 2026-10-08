// Visual check for GH #326: dragging a block bullet into the deep (>50px) zone of
// another block shows the child drop indicator; the shallow strip keeps the
// sibling indicators. jsdom applies no layout, so this drives headless Chromium.
// Usage: npm run build && node scripts/shot-drop-child.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5216;
mkdirSync("screenshots", { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let failed = false;
try {
  for (let i = 0; i < 40; i += 1) {
    try { if ((await fetch(`http://localhost:${PORT}/`)).ok) break; } catch {}
    await sleep(250);
  }
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block .bullet", { timeout: 8000 });
  await sleep(400);
  const blocks = page.locator(".ls-block > .block-main");
  const source = blocks.nth(0).locator(".bullet");
  const targetBlock = page.locator(".ls-block").nth(1);
  const sBox = await source.boundingBox();
  const tBox = await targetBlock.boundingBox();
  const mainBox = await targetBlock.locator("> .block-main").boundingBox();
  await page.mouse.move(sBox.x + sBox.width / 2, sBox.y + sBox.height / 2);
  await page.mouse.down();
  const state = async (x) => {
    await page.mouse.move(x, mainBox.y + mainBox.height / 4, { steps: 6 });
    await sleep(120);
    return page.evaluate(() => ({
      child: !!document.querySelector(".block-main.drop-child"),
      before: !!document.querySelector(".block-main.drop-before"),
      after: !!document.querySelector(".block-main.drop-after"),
    }));
  };
  const shallow = await state(tBox.x + 20);
  console.log("shallow strip", JSON.stringify(shallow));
  if (shallow.child || !shallow.before) failed = true;
  const deep = await state(tBox.x + 120);
  console.log("deep zone   ", JSON.stringify(deep));
  if (!deep.child) failed = true;
  await page.screenshot({ path: "screenshots/drop-child.png" });
  await page.mouse.up();
  await browser.close();
} finally {
  server.kill();
}
console.log(failed ? "FAIL  drop indicator did not follow the zone" : "OK    sibling strip and child zone show their own indicators");
process.exit(failed ? 1 : 0);
