// Browser observation boundary for GH #335 and the GH #129 scaling probe.
// Usage: node scripts/shot-sw-polish.mjs before|after
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import assert from "node:assert/strict";

const phase = process.argv[2] ?? "after";
const out = `artifacts/sw-polish/${phase}`;
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
const observations = [];
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  // Minimal Linux containers have no Han font. An optional evidence-only font
  // fills Arial's Han fallback without changing the app's Latin typography.
  const font = process.env.SW_POLISH_CJK_FONT
    ? readFileSync(process.env.SW_POLISH_CJK_FONT).toString("base64") : null;
  const prepare = async page => {
    if (process.env.SW_POLISH_BASELINE_CSS) await page.evaluate(css => {
      document.querySelector('style[data-vite-dev-id$="/src/styles/app.css"]').textContent = css;
    }, readFileSync(process.env.SW_POLISH_BASELINE_CSS, "utf8"));
    if (font) await page.evaluate(async data => {
      const face = new FontFace("Arial", `url(data:font/otf;base64,${data})`, { unicodeRange: "U+3000-9FFF" });
      document.fonts.add(face);
      await face.load();
    }, font);
    await page.evaluate(() => document.fonts.ready);
  };
  for (const scale of [1, 1.25, 1.5]) {
    const page = await browser.newPage({ viewport: { width: 1000, height: 700 }, deviceScaleFactor: scale });
    await page.goto(`http://localhost:${port}/scripts/fixtures/bullet-threading.html`);
    await page.locator('[data-block-id="focus"] > .block-main .block-content').click();
    await page.evaluate(() => {
      document.documentElement.setAttribute("data-theme", "light");
      window.setThreadSettings({ display: "all", thickness: 2, color: "", "color-by-level": false });
    });
    await prepare(page);
    await page.screenshot({ path: `${out}/threads-${scale}.png` });
    await page.close();
  }
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`http://localhost:${port}/`);
  await page.locator(".nav-all-pages-open").first().waitFor();
  await prepare(page);
  for (const locale of ["en", "zh-CN"]) {
    await page.evaluate(async locale => (await import("/src/i18n/index.ts")).setActiveLocale(locale), locale);
    await page.screenshot({ path: `${out}/sidebar-${locale}.png` });
    observations.push(await page.evaluate(locale => {
      const font = selector => {
        const css = getComputedStyle(document.querySelector(selector));
        return { size: css.fontSize, family: css.fontFamily, weight: css.fontWeight };
      };
      return { locale, allPages: font(".nav-all-pages-open"), sibling: font(".nav-section-toggle") };
    }, locale));
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  for (const row of observations) assert.deepEqual(row.allPages, row.sibling, `${row.locale}: sidebar entry typography`);
  await page.evaluate(async () => (await import("/src/router.ts")).openQueryInNewTab("", "search", true));
  await page.locator(".query-presentations").waitFor();
  for (const locale of ["en", "zh-CN"]) {
    await page.evaluate(async locale => (await import("/src/i18n/index.ts")).setActiveLocale(locale), locale);
    const labels = await page.locator(".query-presentations button").allTextContents();
    assert.deepEqual(labels.map(label => label.trim()), locale === "en" ? ["Search", "List", "Table", "Board"] : ["搜索", "列表", "表格", "看板"]);
    await page.screenshot({ path: `${out}/search-${locale}.png` });
  }
  console.log("Sidebar typography matches in English and Chinese; scale screenshots captured.");
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
