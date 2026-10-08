// Visual regression for GH #88: a whole-block `{{embed ((uuid))}}` presents the
// referenced root as the one interactive root bullet, with a heavier descendant
// guide instead of an extra host bullet or enclosing box.
// Usage: npm run build && node scripts/shot-block-embed.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5206;
const OUT = "screenshots/block-embed-accent";
const CASES = [
  { name: "default-light", theme: "", mode: "light" },
  { name: "default-dark", theme: "", mode: "dark" },
  { name: "nord-dark", theme: "nord", mode: "dark" },
  { name: "solarized-dark", theme: "solarized", mode: "dark" },
  { name: "gruvbox-dark", theme: "gruvbox", mode: "dark" },
];
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

  async function accentMetrics() {
    return host.evaluate((element) => {
      const root = element.querySelector(
        ".embed-block > .live-ref-group > .ls-block",
      );
      const child = root?.querySelector(":scope > .block-children-container > .block-children > .ls-block");
      const rootBullet = root?.querySelector(":scope > .block-main > .block-controls .bullet");
      const childBullet = child?.querySelector(":scope > .block-main > .block-controls .bullet");
      const rootGuide = root?.querySelector(":scope > .block-children-container > .block-children");
      const childGuide = child?.querySelector(":scope > .block-children-container > .block-children");
      if (!rootBullet || !childBullet || !rootGuide || !childGuide) {
        throw new Error("missing embedded root/child accent geometry");
      }
      return {
        rootBullet: getComputedStyle(rootBullet).backgroundColor,
        childBullet: getComputedStyle(childBullet).backgroundColor,
        rootGuide: getComputedStyle(rootGuide).borderLeftColor,
        childGuide: getComputedStyle(childGuide).borderLeftColor,
      };
    });
  }

  function assertAccent(label, metrics) {
    if (metrics.rootBullet !== metrics.rootGuide) {
      throw new Error(`${label}: root bullet ${metrics.rootBullet} and guide ${metrics.rootGuide} drifted`);
    }
    if (metrics.rootBullet === metrics.childBullet) {
      throw new Error(`${label}: embedded root bullet did not differ from ordinary child bullet`);
    }
    if (metrics.rootGuide === metrics.childGuide) {
      throw new Error(`${label}: embedded root guide did not differ from ordinary child guide`);
    }
  }

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

  for (const testCase of CASES) {
    await page.evaluate(({ theme, mode }) => {
      document.documentElement.setAttribute("data-theme", mode);
      window.__tineApplyTheme?.(theme);
    }, testCase);
    await sleep(180);
    const metrics = await accentMetrics();
    assertAccent(testCase.name, metrics);
    await host.screenshot({ path: `${OUT}/${testCase.name}.png` });
    console.log(JSON.stringify({ case: testCase.name, ...metrics }));
  }

  const previousMetrics = await accentMetrics();
  const custom = await page.evaluate(() => {
    const style = document.createElement("style");
    style.id = "block-embed-accent-probe";
    style.textContent = 'html[data-theme="dark"] { --accent: rgb(210, 70, 90); --bullet-color: rgb(90, 100, 110); }';
    document.head.appendChild(style);
    return true;
  });
  if (!custom) throw new Error("could not install custom accent probe");
  const customMetrics = await accentMetrics();
  assertAccent("custom accent", customMetrics);
  if (customMetrics.rootBullet === previousMetrics.rootBullet) {
    throw new Error("custom accent tokens did not recolor the block-embed accent");
  }

  await page.evaluate(() => {
    const style = document.getElementById("block-embed-accent-probe");
    if (style) style.textContent += "\n:root { --block-embed-accent: rgb(1, 2, 3); }";
  });
  const overrideMetrics = await accentMetrics();
  if (overrideMetrics.rootBullet !== "rgb(1, 2, 3)" || overrideMetrics.rootGuide !== "rgb(1, 2, 3)") {
    throw new Error(`custom --block-embed-accent override did not win: ${JSON.stringify(overrideMetrics)}`);
  }
  console.log(JSON.stringify({ out: OUT, bulletCount, hostControlCount, embeddedBulletCount, guideWidth }));
} catch (error) {
  console.error(String(error));
  failed = true;
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
if (failed) process.exit(1);
