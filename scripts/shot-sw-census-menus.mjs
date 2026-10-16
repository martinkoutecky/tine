// UI census 2026-10: shipped components/styles, desktop hover + Android-style tap.
// Usage: source scripts/env.sh && node scripts/shot-sw-census-menus.mjs artifacts/sw-census-menus
import { chromium, devices } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2] ?? "artifacts/sw-census-menus");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
const observations = [];
// Optional fixture font for minimal Linux hosts without CJK system fonts.
const cjkFont = process.env.CENSUS_CJK_FONT
  ? readFileSync(process.env.CENSUS_CJK_FONT).toString("base64") : null;
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const touch of [false, true]) for (const locale of ["en", "zh-CN"]) {
    const mode = touch ? "android" : "desktop";
    const page = await browser.newPage(touch ? { ...devices["Pixel 7"] } : { viewport: { width: 1100, height: 900 } });
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(`http://localhost:${port}/scripts/fixtures/census-menus.html?locale=${locale}`, { timeout: 120_000 });
    if (locale === "zh-CN" && cjkFont) {
      await page.addStyleTag({ content: `@font-face { font-family: CensusCJK; src: url(data:font/otf;base64,${cjkFont}); } :root { --ls-font-family: Inter, CensusCJK, sans-serif; } body, button { font-family: var(--ls-font-family); }` });
    }
    await page.evaluate(() => document.fonts.ready);
    await page.locator(".linked-references .reference-bulk-controls button").waitFor();
    const controls = page.locator(".linked-references .reference-bulk-controls button");
    if (await controls.count() !== 1) throw new Error("Expected one reference bulk button");
    await controls.click();
    if (await page.locator(".linked-references .reference-group-disclosure[aria-expanded=false]").count() !== 2) throw new Error("Reference toggle did not fold both groups");
    await controls.click();
    await page.locator('.linked-references [data-block-id="Alpha"]').waitFor({ state: "visible" });
    await page.screenshot({ path: `${out}/${mode}-${locale}-references.png` });
    for (const target of ["block", "page"]) {
      await page.locator(`#${target}-menu`).click();
      const submenu = page.locator(".ctx-submenu").first();
      if (touch) await submenu.locator(":scope > span").tap(); else await submenu.hover();
      const child = submenu.locator(".ctx-submenu-menu button").first();
      await child.waitFor({ state: "visible" });
      await page.screenshot({ path: `${out}/${mode}-${locale}-${target}-copy-as.png` });
      const box = await child.boundingBox();
      const viewport = page.viewportSize();
      if (!box || box.x < 0 || box.x + box.width > viewport.width) throw new Error("Copy submenu leaves viewport");
      if (target === "page") {
        if (touch) await child.tap(); else await child.click();
        await page.waitForFunction(() => document.querySelector("#copied")?.textContent === "[[Target]]");
      } else await page.keyboard.press("Escape");
      // The generic transient boundary is installed by the full app. This
      // component fixture dismisses the block overlay with its outside click.
      if (await page.locator(".ctx-overlay").count()) await page.locator(".ctx-overlay").click({ position: { x: 2, y: 2 } });
    }
    await page.locator(".qc-actions summary").click({ force: true });
    await page.locator(".qc-menu button").first().waitFor({ state: "visible" });
    const foldLabels = locale === "en" ? ["Collapse all", "Expand all"] : ["全部折叠", "全部展开"];
    const buttons = await page.locator(".qc-menu button").allTextContents();
    if (locale === "en" && buttons.filter(label => foldLabels.includes(label)).length !== 1) throw new Error("Expected one query fold action");
    await page.screenshot({ path: `${out}/${mode}-${locale}-query.png` });
    if (errors.length) throw new Error(errors.join("\n"));
    observations.push({ mode, locale, copyPageReference: "[[Target]]", referenceBulkButtons: 1, queryMenu: buttons });
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  console.log("PASS: desktop hover, touch tap, nested copy action, bulk reference toggle, en + zh-CN");
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
