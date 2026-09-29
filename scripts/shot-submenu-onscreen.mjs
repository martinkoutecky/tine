// GH #471: a context submenu must stay inside the window. Opened over the right
// edge it used to be drawn at `left: 100%` with no idea where the window ended.
// jsdom applies no layout, so this measures the REAL engine over the built app.
// Usage: source scripts/env.sh && npm run build && node scripts/shot-submenu-onscreen.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5214;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

let failed = false;
try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });

  // The reporter's shape: a menu opened at the far right of a row, which is the
  // rightmost menu a user can actually produce here.
  const cases = [
    {
      name: "right-edge",
      viewport: { width: 1100, height: 900 },
      target: ".ls-block:has(.block-children) > .block-main",
      clickAt: (box) => [box.x + box.width - 6, box.y + box.height / 2],
    },
  ];

  for (const testCase of cases) {
    const page = await browser.newPage({ viewport: testCase.viewport, deviceScaleFactor: 2 });
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".block-content", { timeout: 8000 });
    await sleep(400);

    // "Show children as →" needs a block that HAS children.
    const block = page.locator(testCase.target).first();
    await block.waitFor({ timeout: 8000 });
    const box = await block.boundingBox();
    console.log(testCase.name, "row", JSON.stringify({ x: Math.round(box.x), w: Math.round(box.width) }), "vw", testCase.viewport.width);
    const [clickX, clickY] = testCase.clickAt(box);
    await page.mouse.click(clickX, clickY, { button: "right" });
    await page.waitForSelector(".ctx-menu", { timeout: 5000 });

    const item = page.locator(".ctx-submenu").first();
    await item.hover();
    await sleep(250);

    const measured = await page.evaluate(() => {
      const menu = document.querySelector(".ctx-menu");
      const sub = document.querySelector(".ctx-submenu:hover > .ctx-submenu-menu")
        ?? document.querySelector(".ctx-submenu-menu");
      const menuBox = menu.getBoundingClientRect();
      const subBox = sub.getBoundingClientRect();
      return {
        side: menu.getAttribute("data-submenu-side"),
        menu: { left: Math.round(menuBox.left), right: Math.round(menuBox.right) },
        submenu: { left: Math.round(subBox.left), right: Math.round(subBox.right) },
        vw: window.innerWidth,
      };
    });
    const onscreen = measured.submenu.left >= 0 && measured.submenu.right <= measured.vw;
    console.log(testCase.name, JSON.stringify(measured), onscreen ? "ONSCREEN" : "OFFSCREEN");
    if (!onscreen) failed = true;

    await page.screenshot({ path: `${OUT}/ctx-submenu-${testCase.name}.png` });

    // The `over` variant belongs to a viewport too narrow for either side. That
    // layout does not produce a block context menu in this harness, so force the
    // attribute and check the CSS variant itself stays inside the window.
    const over = await page.evaluate(() => {
      const menu = document.querySelector(".ctx-menu");
      menu.setAttribute("data-submenu-side", "over");
      const sub = document.querySelector(".ctx-submenu:hover > .ctx-submenu-menu")
        ?? document.querySelector(".ctx-submenu-menu");
      const menuBox = menu.getBoundingClientRect();
      const subBox = sub.getBoundingClientRect();
      return {
        startsAtItsMenu: Math.abs(subBox.left - menuBox.left) <= 8,
        left: Math.round(subBox.left),
        right: Math.round(subBox.right),
        vw: window.innerWidth,
      };
    });
    const overOnscreen = over.left >= 0 && over.right <= over.vw;
    console.log(`${testCase.name} forced-over`, JSON.stringify(over), overOnscreen ? "ONSCREEN" : "OFFSCREEN");
    if (!overOnscreen) failed = true;

    await page.close();
  }

  console.log(failed ? "FAIL  a submenu left the window" : "OK    every submenu stayed inside the window");
  await browser.close();
} finally {
  server.kill();
}
process.exit(failed ? 1 : 0);
