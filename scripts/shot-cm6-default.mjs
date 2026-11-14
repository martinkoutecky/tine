// Synthetic routed-page proof of the CM6 default and classic opt-out, at phone/desktop widths.
import assert from "node:assert/strict";
import { mkdirSync, realpathSync } from "node:fs";
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";

const out = "target/artifacts/cm6-default-1114/shots";
mkdirSync(out, { recursive: true });
const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("cm6-default-1114"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, fs: { allow: [process.cwd(), realpathSync("node_modules"), realpathSync("node_modules/inter-ui")] } } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const width of [390, 1280]) for (const locale of ["en", "zh-CN"]) {
    const page = await browser.newPage({ viewport: { width, height: 844 }, hasTouch: width === 390 });
    page.setDefaultTimeout(30_000);
    await page.goto(server.resolvedUrls.local[0], { waitUntil: "commit" });
    await page.locator("[data-search-trigger]").waitFor();
    for (const close of await page.locator(".toast-close").all()) await close.click();
    await page.evaluate(async () => (await import("/src/router.ts")).openPage("Tine"));
    await page.locator(".page-title").filter({ hasText: "Tine" }).first().waitFor();
    await page.locator(".main-content .block-content-wrapper").first().click();
    await page.locator('[data-block-editor="cm6"]').waitFor();
    assert.equal(await page.locator("textarea.block-editor").count(), 0);
    await page.keyboard.press("Escape");
    await page.evaluate(async tag => {
      const i18n = await import("/src/i18n/index.ts");
      await i18n.loadLocale(tag); i18n.showLocale(tag);
      (await import("/src/ui.ts")).openSettings("editor");
    }, locale);
    await page.locator(".settings-search-input").fill("CodeMirror");
    await page.locator('[data-setting-target="settings-cm6-label"]').click();
    const field = page.locator('[data-setting-id="settings-cm6-label"]');
    const toggle = field.getByRole("switch");
    assert.equal(await toggle.getAttribute("aria-checked"), "false");
    assert.match(await field.innerText(), locale === "en" ? /Use classic editor/ : /使用经典编辑器/);
    assert.match(await field.innerText(), locale === "en" ? /classic editor is planned to be retired/ : /经典编辑器计划在未来停用/);
    assert.match(await field.innerText(), locale === "en" ? /About → Report an issue/ : /关于 → 报告问题/);
    assert.equal(await field.locator("a").count(), 0);
    await field.scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${out}/${width}-${locale}-default.png` });
    await toggle.click();
    assert.equal(await toggle.getAttribute("aria-checked"), "true");
    await toggle.evaluate(async element => {
      await new Promise(requestAnimationFrame);
      await Promise.all(element.getAnimations({ subtree: true }).map(animation => animation.finished));
    });
    await page.screenshot({ path: `${out}/${width}-${locale}-classic.png` });
    await page.keyboard.press("Escape");
    await page.locator(".main-content .block-content-wrapper").first().click();
    await page.locator("textarea.block-editor").waitFor();
    console.log(`PASS ${width}px ${locale}: default CM6; classic opt-out edits the same routed page`);
    await page.close();
  }
} finally { await browser?.close(); await server.close(); }
