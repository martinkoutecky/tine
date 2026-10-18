// Visual check for GH #417/#418: hovering an embed shows Edit link / Open source /
// Remove just above its top-right corner, clear of the embedded content; a nested
// or neighbouring embed shows only its own. On a touch screen the same toolbar
// appears while a block inside the embed is being edited.
// Usage: npm run build && node scripts/shot-embed-actions.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5209;
const OUT = "screenshots/embed-actions";
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

async function openPage(page, name, ready) {
  await page.keyboard.press("Control+k").catch(() => {});
  if (!(await page.locator(".switcher-input").count())) {
    await page.locator('[aria-label="Search"]').first().click().catch(() => {});
  }
  await page.locator(".switcher-input").fill(name);
  await page.locator(".switcher-row", { hasText: name }).first().click();
  await page.getByText(ready, { exact: true }).first().waitFor({ timeout: 5000 });
  await sleep(300);
}

/** Visible toolbars, and whether each sits above (not over) its embed. */
const toolbars = (page) => page.evaluate(() => [...document.querySelectorAll(".embed-actions")]
  .filter((bar) => getComputedStyle(bar).display !== "none")
  .map((bar) => {
    const embed = bar.parentElement.getBoundingClientRect(), r = bar.getBoundingClientRect();
    return { labels: [...bar.querySelectorAll("button")].map((b) => b.textContent), clearOfContent: r.bottom <= embed.top + 1, rightAligned: Math.abs(r.right - embed.right) <= 1 };
  }));

try {
  mkdirSync(OUT, { recursive: true });
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();

  const desktop = await browser.newPage({ viewport: { width: 900, height: 700 } });
  desktop.setDefaultTimeout(5000);
  const errors = [];
  desktop.on("pageerror", (error) => errors.push(String(error)));
  await desktop.goto(`http://localhost:${PORT}/?regressions`);
  await desktop.waitForSelector(".ls-block", { timeout: 5000 });
  for (const [name, ready, file] of [
    ["Page embed regression", "Source grandchild", "page-embed-hover"],
    ["Block embed regression", "Embedded grandchild", "block-embed-hover"],
  ]) {
    await openPage(desktop, name, ready);
    const embed = desktop.locator(".main-content .embed-block").last();
    const box = await embed.boundingBox();
    // Hover an embedded row's empty right end, as a user reaching for the corner would.
    await desktop.mouse.move(box.x + box.width - 20, box.y + box.height / 2);
    await sleep(200);
    const bars = await toolbars(desktop);
    console.log(file, JSON.stringify(bars));
    if (bars.length !== 1) throw new Error(`${file}: expected exactly one visible toolbar, saw ${bars.length}`);
    if (!bars[0].clearOfContent || !bars[0].rightAligned) throw new Error(`${file}: toolbar is not above the embed's top-right corner`);
    if (bars[0].labels.join("|") !== "Edit link|Open source|Remove") throw new Error(`${file}: unexpected actions ${bars[0].labels}`);
    await desktop.screenshot({ path: `${OUT}/${file}-desktop.png` });
    await desktop.mouse.move(5, 5);
    await sleep(100);
    if ((await toolbars(desktop)).length) throw new Error(`${file}: toolbar stays visible without hover`);
  }

  const phone = await browser.newPage({ viewport: { width: 390, height: 760 }, hasTouch: true, isMobile: true });
  phone.setDefaultTimeout(5000);
  phone.on("pageerror", (error) => errors.push(String(error)));
  await phone.goto(`http://localhost:${PORT}/?regressions`);
  await phone.waitForSelector(".ls-block", { timeout: 5000 });
  await openPage(phone, "Block embed regression", "Embedded grandchild");
  if ((await toolbars(phone)).length) throw new Error("phone: toolbar visible before editing inside the embed");
  await phone.locator(".embed-block .block-content:visible", { hasText: "Embedded child" }).first().tap();
  await phone.waitForSelector(".embed-block textarea.block-editor");
  await sleep(200);
  const bars = await toolbars(phone);
  console.log("phone-editing", JSON.stringify(bars));
  if (bars.length !== 1) throw new Error(`phone: expected the toolbar while editing inside the embed, saw ${bars.length}`);
  await phone.screenshot({ path: `${OUT}/block-embed-editing-phone.png` });
  if (errors.length) throw new Error(`browser errors:\n${errors.join("\n")}`);
} catch (error) {
  console.error(String(error));
  failed = true;
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
if (failed) process.exit(1);
