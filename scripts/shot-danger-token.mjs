// Production QueryMacro in Chromium, plus live theme/custom.css transitions.
// source scripts/env.sh; node scripts/shot-danger-token.mjs <dist> <out> [--check]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";

const [dist, output, check] = process.argv.slice(2);
const out = resolve(output);
mkdirSync(out, { recursive: true });
// Keep generated fixtures with the screenshots. Built CSS lets the same query
// component prove both pre-fix and post-fix rendering without a second checkout.
const urlPath = (path) => "/" + relative(process.cwd(), path);
const assets = resolve(dist, "assets");
const styles = readdirSync(assets).filter((name) => /^(app|main)-.*\.css$/.test(name));
writeFileSync(`${out}/fixture.html`, `<html data-theme="dark"><head>${styles.map((name) => `<link rel="stylesheet" href="${urlPath(`${assets}/${name}`)}">`).join("")}</head><body><div id="root"></div><script type="module" src="${urlPath(`${out}/fixture.tsx`)}"></script></body></html>`);
writeFileSync(`${out}/fixture.tsx`, `
import { render } from "solid-js/web";
import { initParser } from "/src/render/parse";
import { QueryMacro } from "/src/components/Macro";
import { applyTheme, galleryThemes } from "/src/themeGallery";
await initParser();
window.__tineApplyTheme = applyTheme;
window.__tineThemes = [{ id: "", modes: ["light", "dark"] }, ...galleryThemes.map(({ id, modes }) => ({ id, modes }))];
render(() => <main style="max-width:760px;margin:80px auto"><h1>Query diagnostic</h1><QueryMacro body="query (task TODO)" /></main>, document.getElementById("root"));
`);
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("./node_modules/.bin/vite", ["--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    await sleep(100);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on("pageerror", (error) => { errors.push(String(error)); console.error(String(error)); });
  await page.goto(`http://localhost:${port}${urlPath(`${out}/fixture.html`)}`);
  const diagnostic = page.locator(".query-diagnostics").first();
  try {
    await diagnostic.waitFor();
  } catch (error) {
    writeFileSync(`${out}/failure.txt`, JSON.stringify({ journey: "production query diagnostic", expected: "visible diagnostic", observed: await page.locator("body").innerText(), errors, classification: "harness" }, null, 2));
    await page.screenshot({ path: `${out}/failure.png` });
    throw error;
  }
  const measure = () => diagnostic.evaluate((element) => {
      const style = getComputedStyle(element);
      let host = element;
      let background;
      do {
        background = getComputedStyle(host).backgroundColor;
        host = host.parentElement;
      } while (host && background === "rgba(0, 0, 0, 0)");
      const rgb = (value) => value.match(/[\d.]+/g).slice(0, 3).map(Number);
      const luminance = (value) => rgb(value).map((c) => c / 255).map((c) => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4).reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
      const [lo, hi] = [luminance(style.color), luminance(background)].sort((a, b) => a - b);
      return { text: element.textContent, foreground: style.color, background, contrast: (hi + 0.05) / (lo + 0.05) };
  });
  const metrics = {};
  for (const theme of ["", "soft"]) {
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = "dark";
      window.__tineApplyTheme(theme);
    }, theme);
    await diagnostic.scrollIntoViewIfNeeded();
    await sleep(200);
    const name = theme || "default";
    metrics[name] = await measure();
    await page.screenshot({ path: `${out}/${name}-dark.png` });
    if (check) assert.ok(metrics[name].contrast >= 4.5, JSON.stringify(metrics[name]));
  }
  if (check) {
    metrics.themes = {};
    const themes = await page.evaluate(() => window.__tineThemes);
    for (const { id, modes } of themes) for (const mode of modes) {
      await page.evaluate(([theme, mode]) => {
        document.documentElement.dataset.theme = mode;
        window.__tineApplyTheme(theme);
      }, [id, mode]);
      const measured = await measure();
      metrics.themes[`${id || "default"}-${mode}`] = measured;
      assert.ok(measured.contrast >= 4.5, JSON.stringify(measured));
    }
    // Same specificity a user writes in custom.css; survives live mode/palette switches.
    await page.addStyleTag({ content: "html { --tine-danger-color: rgb(255, 200, 210); }" });
    for (const { id: theme, modes } of themes) for (const mode of modes) {
      await page.evaluate(([theme, mode]) => {
        document.documentElement.dataset.theme = mode;
        window.__tineApplyTheme(theme);
      }, [theme, mode]);
      assert.equal(await diagnostic.evaluate((el) => getComputedStyle(el).color), "rgb(255, 200, 210)");
    }
  }
  assert.deepEqual(errors, []);
  writeFileSync(`${out}/metrics.json`, JSON.stringify(metrics, null, 2) + "\n");
  console.log(JSON.stringify(metrics));
} finally {
  await browser?.close();
  server.kill();
}
