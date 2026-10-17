// Visual check for GH #439: a whole-page `{{embed [[Page]]}}` sits on the outline
// grid of its host (embedded roots line up with the host's own children) and names
// the embedded page.
// Usage: npm run build && node scripts/shot-page-embed.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5207;
const OUT = "screenshots/page-embed";
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
  mkdirSync(OUT, { recursive: true });
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();
  for (const viewport of [
    { name: "desktop", opts: { viewport: { width: 900, height: 700 } } },
    { name: "phone", opts: { viewport: { width: 390, height: 760 }, hasTouch: true, isMobile: true } },
  ]) {
    const page = await browser.newPage(viewport.opts);
    page.setDefaultTimeout(5000);
    const errors = [];
    page.on("console", (message) => message.type() === "error" && errors.push(message.text()));
    page.on("pageerror", (error) => errors.push(String(error)));
    await page.goto(`http://localhost:${PORT}/?regressions`);
    await page.waitForSelector(".ls-block", { timeout: 5000 });
    await page.evaluate(() => { window.location.hash = ""; });
    await page.keyboard.press("Control+k").catch(() => {});
    if (!(await page.locator(".switcher-input").count())) {
      await page.locator('[aria-label="Search"]').first().click().catch(() => {});
    }
    await page.locator(".switcher-input").fill("Page embed regression");
    await page.locator(".switcher-row").first().click();
    await page.getByText("Source grandchild", { exact: true }).first().waitFor({ timeout: 5000 });
    await sleep(300);
    const geometry = await page.evaluate(() => {
      const left = (el) => el?.getBoundingClientRect().left ?? null;
      const bullet = (block) => block?.querySelector(":scope > .block-main .bullet");
      const blocks = [...document.querySelectorAll(".ls-block")];
      const parent = blocks.find((b) => b.textContent?.startsWith("Ordinary parent"));
      const sibling = blocks.find((b) => b.textContent?.startsWith("Ordinary sibling"));
      const indent = left(bullet(sibling)) - left(bullet(parent));
      return [...document.querySelectorAll(".embed-block.embed-page")].map((embed) => {
        const host = embed.closest(".ls-block");
        const root = embed.querySelector(":scope > .live-ref-group > .ls-block");
        return {
          title: embed.querySelector(".embed-page-header")?.textContent ?? null,
          expectedRootBullet: left(bullet(host)) + indent,
          rootBullet: left(bullet(root)),
        };
      });
    });
    console.log(viewport.name, JSON.stringify(geometry));
    if (geometry.length !== 2) throw new Error(`${viewport.name}: expected 2 page embeds, saw ${geometry.length}`);
    for (const g of geometry) {
      if (!g.title?.includes("Page embed source regression")) throw new Error(`${viewport.name}: embed header does not name the page: ${g.title}`);
      if (Math.abs(g.rootBullet - g.expectedRootBullet) > 1) {
        throw new Error(`${viewport.name}: embedded root bullet at ${g.rootBullet}, host children grid at ${g.expectedRootBullet}`);
      }
    }
    await page.screenshot({ path: `${OUT}/page-embed-${viewport.name}.png` });
    if (errors.length) throw new Error(`browser errors:\n${errors.join("\n")}`);
    await page.close();
  }
} catch (error) {
  console.error(String(error));
  failed = true;
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
if (failed) process.exit(1);
