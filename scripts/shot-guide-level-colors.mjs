// Level-colored indent guides acceptance shots (GH #856). A page nested eight
// levels deep, light and dark, the knob on (default) and off, one zoomed-in block
// and an embed inside a nested block. Also checks the colors: each level's guide
// differs from its neighbours, the cycle repeats every six levels, the knob off
// paints one color, and an embed starts again at level 1. Headless Chromium over
// the mock backend.
// Usage (after `source scripts/env.sh && npx vite build`):
//   node scripts/shot-guide-level-colors.mjs <distDir> <outDir>
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { resolve } from "node:path";

const [, , distArg = "dist", outArg = "artifacts/og-guide-level-colors"] = process.argv;
const dist = resolve(distArg);
const out = resolve(outArg);
mkdirSync(out, { recursive: true });
const PORT = Number(process.env.SHOT_PORT) || 20000 + Math.floor(Math.random() * 20000);
const server = spawn("npx", ["vite", "preview", "--outDir", dist, "--port", String(PORT), "--strictPort"], { stdio: "ignore", detached: true });
const stop = () => { try { process.kill(-server.pid, "SIGTERM"); } catch {} };

async function waitForServer(url) {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}
const fail = (message) => { throw new Error(message); };

/** The guide color of each `.block-children` along the path to the block whose text starts with `text`. */
const guideColorsTo = (page, text) => page.evaluate((needle) => {
  const leaf = [...document.querySelectorAll(".block-content")].find((el) => el.textContent.trim().startsWith(needle));
  const colors = [];
  for (let el = leaf; el; el = el.parentElement) {
    if (el.classList?.contains("block-children")) colors.unshift(getComputedStyle(el).borderInlineStartColor);
  }
  return colors;
}, text);

async function openPage(page, title) {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill(title);
  await sleep(300);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(400);
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const mode of ["light", "dark"]) {
    const context = await browser.newContext({ viewport: { width: 1100, height: 760 }, deviceScaleFactor: 2 });
    const page = await context.newPage();
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/?regressions`);
    await page.waitForSelector(".page-title", { timeout: 10000 });
    await sleep(400);
    await page.evaluate((m) => document.documentElement.setAttribute("data-theme", m), mode);

    await openPage(page, "Guide level colors regression");
    await page.getByText("Level 7 leaf", { exact: true }).waitFor({ timeout: 8000 });
    await sleep(300);
    const knobsCss = await page.evaluate(() => document.getElementById("tine-knobs")?.textContent ?? "");
    if (knobsCss !== "") fail(`${mode}: an untouched knob must emit nothing, got ${JSON.stringify(knobsCss)}`);
    await page.screenshot({ path: `${out}/levels-on-${mode}.png` });

    const colors = await guideColorsTo(page, "Level 7 leaf");
    console.log(`${mode}: guide colors by level`, colors);
    if (colors.length !== 7) fail(`${mode}: expected 7 guides above the level 7 leaf, got ${colors.length}`);
    for (let i = 1; i < colors.length; i++) if (colors[i] === colors[i - 1]) fail(`${mode}: levels ${i} and ${i + 1} share ${colors[i]}`);
    if (new Set(colors.slice(0, 6)).size !== 6) fail(`${mode}: the first six levels must be six different colors`);
    if (colors[6] !== colors[0]) fail(`${mode}: level 7 must repeat level 1 (${colors[6]} vs ${colors[0]})`);

    // Zoomed in: the block's own children are level 1 again.
    await page.locator(".ls-block", { has: page.getByText("Level 2 first", { exact: true }) }).first()
      .locator(".bullet-container").first().click();
    await page.mouse.move(2, 2);
    await sleep(500);
    await page.screenshot({ path: `${out}/levels-zoomed-${mode}.png` });
    const zoomed = await guideColorsTo(page, "Level 3 first");
    console.log(`${mode}: zoomed (Level 2 first) guides above "Level 3 first"`, zoomed);
    if (zoomed[0] !== colors[0]) fail(`${mode}: zoomed-in children must start at level 1 (${zoomed[0]} vs ${colors[0]})`);
    await page.keyboard.press("Alt+ArrowLeft");
    await sleep(300);

    // Embedded in a nested block: the embedded outline starts again at level 1 (accent line there).
    await openPage(page, "Guide level colors embed regression");
    await page.getByText("Level 7 leaf", { exact: true }).first().waitFor({ timeout: 8000 });
    await sleep(300);
    await page.screenshot({ path: `${out}/levels-embed-${mode}.png` });
    const embedded = await page.evaluate(() => {
      const leaf = [...document.querySelectorAll(".block-embed-host .block-content")].find((el) => el.textContent.trim().startsWith("Level 4 first"));
      const colors = [];
      for (let el = leaf; el && !el.classList?.contains("embed-block"); el = el.parentElement) {
        if (el.classList?.contains("block-children")) colors.unshift(getComputedStyle(el).borderInlineStartColor);
      }
      return colors;
    });
    console.log(`${mode}: guides inside the embed above "Level 4 first"`, embedded);
    // Inside the embed: guide 1 is the accent line; guides 2.. continue at levels 2.. from the embed's own root.
    if (embedded.length < 3 || embedded[1] !== colors[1] || embedded[2] !== colors[2]) fail(`${mode}: the embed must count from its own root`);

    // Knob off: today's single guide color.
    await page.locator('button.icon-btn[title^="Settings"]').first().click();
    await page.waitForSelector(".settings-modal", { timeout: 3000 });
    await sleep(300);
    const appearance = page.locator(".settings-modal").getByText("Appearance", { exact: true }).first();
    if (await appearance.count()) await appearance.click();
    await sleep(200);
    const advanced = page.locator(".settings-advanced-toggle");
    if (await advanced.count()) await advanced.first().click();
    await sleep(200);
    const knob = page.locator('select[aria-label="Color indent guides by level"]');
    await knob.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await sleep(200);
    await page.screenshot({ path: `${out}/settings-knob-${mode}.png` });
    await knob.selectOption("single");
    await sleep(200);
    await page.locator(".settings-modal").press("Escape");
    await page.locator(".settings-modal").waitFor({ state: "detached", timeout: 3000 }).catch(async () => {
      await page.keyboard.press("Escape");
      await page.locator(".settings-modal").waitFor({ state: "detached", timeout: 3000 });
    });
    await sleep(300);
    const cls = await page.evaluate(() => [...document.documentElement.classList].filter((c) => c.startsWith("tine-")));
    if (cls.join() !== "tine-guide-colors-single") fail(`${mode}: expected the single class, got ${cls}`);
    await openPage(page, "Guide level colors regression");
    await page.getByText("Level 7 leaf", { exact: true }).waitFor({ timeout: 8000 });
    await sleep(300);
    await page.screenshot({ path: `${out}/levels-off-${mode}.png` });
    const off = await guideColorsTo(page, "Level 7 leaf");
    if (new Set(off).size !== 1) fail(`${mode}: knob off must paint one color, got ${[...new Set(off)]}`);
    console.log(`OK    ${mode}: knob off paints ${off[0]}`);
    await context.close();
  }
  await browser.close();
  console.log("done");
} finally {
  stop();
}
