// UI-HLC-INLINE-COLOURS: the selection toolbar's row of highlight colours.
// Real browser app on the in-memory `?regressions` mock graph ("Highlight colour regression"),
// in light and dark, in both editors: select words, check the second row, press a colour, check
// the written markup, leave the block and check the rendered mark's computed background is
// Logseq's step-5 colour for the theme (the variable resolves; the fallback is not used).
// Run: node scripts/shot-highlight-colors.mjs [artifactDir]
// Fail-before: develop has no `.sel-toolbar-colors`, so the first wait fails.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { chromium } from "playwright";

const out = process.argv[2] ?? "target/artifacts/highlight-colors";
const commit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
mkdirSync(out, { recursive: true });
const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("highlight-colors"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, watch: { ignored: ["**/target/**", "**/.toolchain/**", "**/.cargo/**"] } },
});
await server.listen();
// Logseq's --rx-*-05 (OG 6e7afa8eb packages/ui/src/radix.css) as Chromium computes them.
const expected = { light: { blue: "rgb(206, 231, 254)", red: "rgb(253, 216, 216)" }, dark: { blue: "rgb(15, 48, 88)", red: "rgb(84, 27, 31)" } };
const results = [], failures = [], browserErrors = [];
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const theme of ["light", "dark"]) for (const engine of ["cm6", "textarea"]) {
    const tag = `${theme}-${engine}`;
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.setDefaultTimeout(15_000);
    page.on("pageerror", (error) => browserErrors.push(`${tag}: ${error}`));
    await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
    await page.goto(`${server.resolvedUrls.local[0]}?regressions`);
    await page.locator(".ls-block").first().waitFor();
    await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await page.addStyleTag({ content: ".toast-stack { display: none !important; }" });
    await page.evaluate(async (on) => (await import("/src/editor/cm6Setting.ts")).setCm6EditorEnabled(on), engine === "cm6");
    if (engine === "cm6") await page.evaluate(async () => { await import("/src/components/Cm6BlockEditor.tsx"); });
    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill("Highlight colour regression");
    await page.locator(".switcher-row").filter({ hasText: "Highlight colour regression" }).first().click();
    await page.locator(".page-title").filter({ hasText: "Highlight colour regression" }).first().waitFor();
    await page.evaluate(() => document.fonts.ready);
    try {
      // The existing red mark resolves to the theme's red.
      const existing = await page.locator(".page-blocks mark").filter({ hasText: /^red$/ }).first()
        .evaluate((m) => getComputedStyle(m).backgroundColor);
      assert.equal(existing, expected[theme].red, `${tag}: existing red mark computes ${existing}`);
      const block = page.locator(".page-blocks .block-content").filter({ hasText: "Colour these words here please" }).first();
      await block.click({ position: { x: 4, y: 8 } });
      await page.locator(engine === "cm6" ? ".block-main.editing .cm-content" : ".block-main.editing textarea").first().waitFor();
      await page.keyboard.press("Home");
      for (let i = 0; i < 7; i++) await page.keyboard.press("ArrowRight");
      for (let i = 0; i < 11; i++) await page.keyboard.press("Shift+ArrowRight");
      const row = page.locator(".sel-toolbar .sel-toolbar-colors");
      await row.waitFor();
      const box = await page.locator(".sel-toolbar").first().boundingBox();
      await page.screenshot({ path: `${out}/${tag}-toolbar.png`, clip: { x: Math.max(0, box.x - 40), y: Math.max(0, box.y - 20), width: 520, height: box.height + 90 } });
      const swatches = await row.locator("[data-highlight-color]").evaluateAll((els) => els.map((el) => [el.dataset.highlightColor, getComputedStyle(el).backgroundColor]));
      assert.deepEqual(swatches.map(([c]) => c), ["yellow", "red", "pink", "green", "blue", "purple", "gray", ""], `${tag}: swatches`);
      assert.equal(swatches[4][1], expected[theme].blue, `${tag}: blue swatch computes ${swatches[4][1]}`);
      await row.locator('[data-highlight-color="blue"]').click();
      const raw = await page.waitForFunction(async () => {
        const { doc } = await import("/src/document/model.ts");
        return Object.values(doc.byId).find((n) => n.raw?.includes("these words"))?.raw.includes("<mark") ? Object.values(doc.byId).find((n) => n.raw.includes("these words")).raw : null;
      }).then((h) => h.jsonValue());
      assert.equal(raw, 'Colour <mark style="background-color: var(--ls-highlight-color-blue, #cee7fe)">these words</mark> here please', `${tag}: raw`);
      await page.keyboard.press("Escape");
      const mark = page.locator(".page-blocks .block-content mark").filter({ hasText: /^these words$/ }).first();
      await mark.waitFor();
      const rendered = await mark.evaluate((m) => ({ bg: getComputedStyle(m).backgroundColor, color: getComputedStyle(m).color }));
      assert.equal(rendered.bg, expected[theme].blue, `${tag}: rendered mark computes ${rendered.bg}`);
      await page.screenshot({ path: `${out}/${tag}-rendered.png`, clip: { x: 200, y: 80, width: 900, height: 260 } });
      results.push({ tag, existing, swatches, raw, rendered });
    } catch (error) {
      failures.push(String(error.message ?? error));
      await page.screenshot({ path: `${out}/${tag}-failure.png` }).catch(() => {});
    }
    await ctx.close();
  }
} finally {
  writeFileSync(`${out}/evidence.json`, JSON.stringify({ commit, results, failures, browserErrors }, null, 2));
  await browser?.close();
  await server.close();
}
if (failures.length || browserErrors.length) {
  console.error([...failures, ...browserErrors].join("\n"));
  process.exit(1);
}
console.log(`highlight colours: ${results.length} runs passed; screenshots in ${out}`);
