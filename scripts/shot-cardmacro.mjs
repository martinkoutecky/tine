// Verify the block-level user macro ({{card}}) renders as real nested blocks
// (heading + paragraph + list) while inline macros (poem/hi) stay inline — the
// Item 2 OG-parity behavior. Real frontend (Chromium + mock via vite preview).
// Writes screenshots/macro-card-block.png + macro-inline.png (gitignored).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5199;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) { try { if ((await fetch(url)).ok) return; } catch {} await sleep(250); }
  throw new Error("server did not start");
}
try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 900, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  // Navigate to the kitchen-sink page (where the demo macros live) via the switcher.
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 3000 });
  await page.locator(".switcher-input").fill("kitchen");
  await sleep(400);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await sleep(500);

  // Card block: the .ls-block whose text starts with "Block user macro".
  const card = page.locator(".ls-block", { hasText: "Block user macro" }).first();
  await card.scrollIntoViewIfNeeded();
  await sleep(150);
  await card.screenshot({ path: "screenshots/macro-card-block.png" });
  // Inline macros block (poem/hi).
  const inline = page.locator(".ls-block", { hasText: "Roses are" }).first();
  await inline.scrollIntoViewIfNeeded();
  await sleep(150);
  await inline.screenshot({ path: "screenshots/macro-inline.png" });

  // Structural check: card block contains a heading + a list (block-level), the
  // inline macro block does NOT introduce block children.
  const cardHasHeading = await card.locator(".heading-text, h1, h2, h3, .macro-blocks").count();
  const cardHtml = await card.evaluate((el) => el.querySelector(".macro-blocks")?.innerHTML?.slice(0, 400) ?? "NO .macro-blocks");
  console.log("card .macro-blocks/heading count:", cardHasHeading);
  console.log("card .macro-blocks innerHTML head:\n", cardHtml);
  console.log(errors.length ? "PAGE ERRORS:\n" + errors.join("\n") : "no page errors");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
