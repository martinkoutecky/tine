// tine#823: screenshots of the interface. usage: node scripts/shot-rtl-chrome.mjs <out-dir> [rtl|fa|zh]
// With `rtl`, installs the test-only pseudo right-to-left locale; with `fa`, shows the shipped Persian
// catalog. Both check that the chrome mirrors. `zh` is a Simplified Chinese browser locale.
// Without it, the screenshots are the left-to-right baseline, for a pixel comparison between two trees.
import { spawn } from "node:child_process";
import { closeSync, mkdirSync, openSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { chromium } from "playwright";

const out = resolve(process.argv[2] || "target/artifacts/rtl-chrome"), mode = process.argv[3], rtl = mode === "rtl" || mode === "fa";
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const log = openSync(`${out}/vite.log`, "w");
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: ["ignore", log, log], detached: true });
closeSync(log);
const failures = [];
let browser;
try {
  for (let a = 0; ; a++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (a === 600) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}), args: ["--no-sandbox", "--disable-gpu", "--force-color-profile=srgb"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, locale: mode === "zh" ? "zh-CN" : mode === "fa" ? "fa-IR" : "en-US" });
  page.on("pageerror", e => failures.push(`page error: ${e.message}`));
  await page.clock.install({ time: new Date("2026-11-01T10:00:00Z") });
  await page.goto(`http://localhost:${port}/`, { timeout: 120_000 });
  await page.waitForSelector(".ls-block, .page-title", { timeout: 60_000 });
  await page.evaluate(() => document.fonts.ready);
  await page.clock.runFor(2000);
  if (mode === "rtl") await page.evaluate(async () => {
    const i18n = await import("/src/i18n/index.ts");
    i18n.installPseudoRtlLocale("xx-Rtl", ["settings-ui-logical-outdenting = فارسی\n"]);
    i18n.showLocale("xx-Rtl");
  });
  await page.addStyleTag({ content: "*,*::before,*::after{transition:none!important;animation:none!important;caret-color:transparent!important}" });
  await page.clock.runFor(1000);
  const shot = name => page.screenshot({ path: `${out}/${name}.png` });
  await shot("main");
  if (rtl) {
    const box = await page.evaluate(() => {
      const rect = s => document.querySelector(s)?.getBoundingClientRect() ?? null;
      return { sidebar: rect(".left-sidebar, .sidebar"), main: rect(".main-content"), width: innerWidth, dir: document.documentElement.getAttribute("dir"), bullet: rect(".ls-block .bullet-container"), body: rect(".ls-block .block-content") };
    });
    if (box.dir !== "rtl") failures.push("the root is not right to left");
    if (box.sidebar && box.main && box.sidebar.left < box.main.left) failures.push("the left sidebar did not move to the right edge");
    if (box.bullet && box.body && box.bullet.left < box.body.right - 1) failures.push("the note layout did not default to right to left");
  }
  const settingsTitle = await page.evaluate(async () => (await import("/src/i18n/index.ts")).t("app-settings-t-s"));
  await page.getByTitle(settingsTitle, { exact: true }).click();
  await page.waitForSelector(".settings-modal");
  await page.clock.runFor(800); await delay(600);
  await shot("settings");
  await page.keyboard.press("Escape"); await page.clock.runFor(300); await delay(200);
  await page.keyboard.press("Control+k"); await page.clock.runFor(300); await delay(400);
  await page.waitForSelector(".switcher-input");
  await shot("search");
} finally {
  await browser?.close();
  try { process.kill(-server.pid); } catch {}
}
if (failures.length) { console.error(failures.join("\n")); process.exit(1); }
console.log(`rtl-chrome: ok, ${out}`);
