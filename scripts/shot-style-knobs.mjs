// Style knobs acceptance shots (GH #610 item 3): the Settings > Appearance >
// Fine-tuning section, and the embed page before / after choosing "Accent line
// and dot only". Headless Chromium over the mock backend.
// Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-style-knobs.mjs <distDir> <outDir>
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { resolve } from "node:path";

const [, , distArg = "dist", outArg = "subagent-tasks/notes/style-knobs"] = process.argv;
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

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const mode of ["light", "dark"]) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
    const page = await context.newPage();
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/?regressions`);
    await page.waitForSelector(".page-title", { timeout: 10000 });
    await sleep(400);
    await page.evaluate((m) => document.documentElement.setAttribute("data-theme", m), mode);

    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill("Block embed regression");
    await page.locator(".switcher-row").first().click();
    await page.locator(".block-embed-host[data-block-id]").getByText("Embedded grandchild", { exact: true }).waitFor({ timeout: 8000 });
    await sleep(300);
    const shade = () => page.evaluate(() => getComputedStyle(document.querySelector(".embed-block")).backgroundColor);
    const before = await shade();
    await page.screenshot({ path: `${out}/embed-default-${mode}.png` });
    if (before === "rgba(0, 0, 0, 0)") fail(`${mode}: the default embed should be shaded, got ${before}`);
    const knobsBefore = await page.evaluate(() => document.getElementById("tine-knobs")?.textContent ?? null);
    if (knobsBefore !== "") fail(`${mode}: untouched knobs must leave #tine-knobs empty, got ${JSON.stringify(knobsBefore)}`);

    await page.locator('button.icon-btn[title^="Settings"]').first().click();
    await page.waitForSelector(".settings-modal", { timeout: 3000 });
    await sleep(300);
    const heading = page.locator(".settings-section", { hasText: "Fine-tuning" });
    await heading.evaluate((el) => el.scrollIntoView({ block: "start" }));
    await sleep(200);
    await page.screenshot({ path: `${out}/settings-fine-tuning-${mode}.png` });

    await page.locator('select[aria-label="Embedded blocks"]').selectOption("accent");
    await page.locator('select[aria-label="Links to the web or a file"]').selectOption("none");
    await sleep(200);
    await page.screenshot({ path: `${out}/settings-fine-tuning-chosen-${mode}.png` });
    await page.keyboard.press("Escape");
    await sleep(300);
    const after = await shade();
    await page.screenshot({ path: `${out}/embed-accent-${mode}.png` });
    if (after !== "rgba(0, 0, 0, 0)") fail(`${mode}: accent-only embeds must not be shaded, got ${after}`);
    const css = await page.evaluate(() => document.getElementById("tine-knobs").textContent);
    if (css !== ":root { --tine-external-link-underline: none; }") fail(`${mode}: unexpected #tine-knobs text ${JSON.stringify(css)}`);
    console.log(`OK    ${mode}: embed ${before} -> ${after}`);
    await context.close();
  }
  await browser.close();
  console.log("done");
} finally {
  stop();
}
