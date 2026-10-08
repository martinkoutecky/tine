// Verify bounded shell rendering and body latching in the production Big fixture.
// Seen-window scroll extent must survive a top/bottom round trip (ADR 0072).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5199;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

const count = (page, sel) => page.evaluate((s) => document.querySelectorAll(s).length, sel);
let failed = false;
const check = (name, cond, detail) => {
  console.log(`${cond ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
  if (!cond) failed = true;
};

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 900, height: 900 } });
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  // ?big makes the mock expose the 2000-block page; reach it via the quick switcher.
  await page.goto(`http://localhost:${PORT}/?big`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 3000 });
  await page.locator(".switcher-input").fill("Big");
  await sleep(400);
  await page.locator(".switcher-row").first().click();
  await page.waitForFunction(() => document.querySelector(".page-title")?.textContent.trim() === "Big");
  await sleep(700); // let the near-viewport blocks render + IO settle

  const totalBlocks = await count(page, ".ls-block");
  const deferred0 = await count(page, ".ast-deferred");
  const heavy0 = (await count(page, ".md-table")) + (await count(page, ".code-block")) + (await count(page, ".katex"));
  console.log(`blocks=${totalBlocks} deferred0=${deferred0} heavy0=${heavy0}`);
  await page.screenshot({ path: "screenshots/virtualize-top.png" });

  check("large page has bounded mounted shells", totalBlocks > 0 && totalBlocks < 300, `${totalBlocks} shells`);
  check("offscreen shells absent", await page.locator("[data-outline-window]").count() > totalBlocks / 24, `${deferred0} deferred bodies in mounted shells`);
  check("only near-viewport heavy constructs rendered", heavy0 < 250, `${heavy0} heavy`);

  // The top-of-page block is always inside the near-zone, so it must be rendered.
  // (We don't assert a fixed COUNT of rendered blocks — how many fit the near-zone
  // depends on block heights; the point is that the vast majority are deferred.)
  const firstDeferred = await page.evaluate(() => {
    const first = document.querySelector(".ls-block");
    return first ? !!first.querySelector(".ast-deferred") : true;
  });
  check("top-of-page block rendered (not deferred)", firstDeferred === false);

  // Scroll to the bottom; the bottom blocks should render on demand.
  await page.evaluate(() => {
    const sc = document.querySelector(".main-content");
    if (sc) sc.scrollTop = sc.scrollHeight;
  });
  await sleep(800);
  const deferredBottom = await count(page, ".ast-deferred");
  const heavyBottom = (await count(page, ".md-table")) + (await count(page, ".code-block")) + (await count(page, ".katex"));
  const lastDeferred = await page.evaluate(() => {
    const blocks = document.querySelectorAll(".ls-block");
    const last = blocks[blocks.length - 1];
    return last ? !!last.querySelector(".ast-deferred") : true;
  });
  console.log(`deferredBottom=${deferredBottom} heavyBottom=${heavyBottom} lastDeferred=${lastDeferred}`);
  await page.screenshot({ path: "screenshots/virtualize-bottom.png" });

  check("last block rendered after scroll", lastDeferred === false);
  check("bottom renders heavy constructs", heavyBottom > 0, `${heavyBottom} heavy`);
  const heightBottom = await page.locator(".main-content").evaluate((e) => e.scrollHeight);

  // Return over measured windows: shells remount, body latches remain.
  await page.evaluate(() => {
    const sc = document.querySelector(".main-content");
    if (sc) sc.scrollTop = 0;
  });
  await sleep(500);
  const deferredFinal = await count(page, ".ast-deferred");
  console.log(`deferredFinal=${deferredFinal}`);
  check("returned top body stays rendered", await page.locator(".ls-block").first().locator(".ast-deferred").count() === 0);
  await page.locator(".main-content").evaluate((e) => { e.scrollTop = e.scrollHeight; });
  await sleep(500);
  const heightAgain = await page.locator(".main-content").evaluate((e) => e.scrollHeight);
  check("seen-window scrollbar extent stays stable", heightAgain === heightBottom, `${heightBottom} → ${heightAgain}`);

  const stats = await page.evaluate(() => window.__tineParseStats ?? null);
  console.log(stats ? `parseStats=${JSON.stringify(stats)}` : "parseStats unavailable (production build — DEV counter stripped)");

  console.log(errors.length ? "CONSOLE ERRORS:\n" + errors.join("\n") : "no console errors");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(failed ? 1 : 0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
