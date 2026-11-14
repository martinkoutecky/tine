// Synthetic routed-page Settings journey and phone/desktop visual proof for #926.
// SETTINGS_926_BEFORE=1 captures the current phone layout before implementation.
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { mkdirSync, realpathSync } from "node:fs";
import assert from "node:assert/strict";

const out = "target/artifacts/settings-926/shots";
mkdirSync(out, { recursive: true });
const before = !!process.env.SETTINGS_926_BEFORE;
const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("settings-926"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, fs: { allow: [process.cwd(), realpathSync("node_modules"), realpathSync("node_modules/inter-ui")] } } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const width of before ? [390] : [390, 1280]) {
    const page = await browser.newPage({ viewport: { width, height: 844 }, hasTouch: width === 390 });
    page.setDefaultTimeout(15000);
    const url = new URL(server.resolvedUrls.local[0]);
    if (width === 390) url.searchParams.set("platform", "ios");
    await page.goto(url.href, { waitUntil: "commit" });
    await page.locator("[data-search-trigger]").waitFor({ timeout: 60000 });
    await page.evaluate(async () => (await import("/src/router.ts")).openPage("Tine"));
    await page.locator(".page-title").filter({ hasText: "Tine" }).first().waitFor();
    for (const close of await page.locator(".toast-close").all()) await close.click();
    await page.getByTitle(/^Settings/).first().click();
    await page.locator(".settings-modal").waitFor();
    const shot = async name => page.screenshot({ path: `${out}/${before ? "before" : width}-${name}.png` });
    await shot("list");
    if (before) { await page.close(); continue; }
    if (width === 390) {
      assert.equal(await page.locator('[data-setting-id="settings-ui-mode"]').count(), 0);
      const bounds = await page.locator(".settings-modal").boundingBox();
      assert.ok(bounds.x <= 1 && bounds.y <= 1 && bounds.width >= 389 && bounds.height >= 843, JSON.stringify(bounds));
    }
    for (const [label, name] of [["Appearance", "category"], ["Themes", "themes"], ["Keyboard shortcuts", "shortcuts"]]) {
      await page.locator(".settings-nav-item").filter({ hasText: new RegExp(`^${label}$`) }).click();
      if (name === "themes") await page.locator(".theme-gallery-thumb img").evaluateAll(images => Promise.all(images.map(image => image.decode())));
      if (name === "shortcuts") {
        const search = page.locator(".settings-search-input");
        await search.fill("go to home");
        await page.locator('[data-shortcut-id="go/home"]').waitFor();
        if (width === 390) {
          const row = page.locator('[data-shortcut-id="go/home"]');
          const labelBox = await row.locator(".help-shortcut-label").boundingBox();
          const unbindBox = await row.getByRole("button", { name: "Unbind", exact: true }).boundingBox();
          assert.ok(Math.abs(labelBox.y - unbindBox.y) < Math.max(labelBox.height, unbindBox.height), "description and Unbind share a row");
          assert.ok(await row.locator(".help-keycap-button").isVisible());
        }
      }
      await shot(name);
      if (width === 390) await page.getByRole("button", { name: "Back to Settings", exact: true }).click();
    }
    if (width !== 390) await page.locator(".settings-nav-item").filter({ hasText: /^Themes$/ }).click();
    await page.locator(".settings-search-input").fill("color scheme");
    await page.locator('[data-setting-target="themes-color-scheme"]').click();
    await page.locator('[data-setting-id="themes-color-scheme"]').waitFor();
    if (width === 390) {
      await page.evaluate(() => {
        document.documentElement.style.setProperty("--overlay-inset-top", "48px");
        document.documentElement.style.setProperty("--overlay-inset-bottom", "34px");
      });
      const bounds = await page.locator(".settings-modal").boundingBox();
      assert.equal(bounds.y, 48);
      assert.equal(Math.round(bounds.y + bounds.height), 810);
      await shot("safe-area");
    }
    await page.getByRole("button", { name: "Close Settings", exact: true }).click();
    await page.locator(".settings-modal").waitFor({ state: "detached" });
    await page.close(); console.log(`PASS Settings journey at ${width}px`);
  }
} finally { await browser?.close(); await server.close(); }
