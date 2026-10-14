import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import assert from "node:assert/strict";

const out = "artifacts/mobile-2";
fs.mkdirSync(out, { recursive: true });
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", "5322", "--strictPort"], { stdio: "pipe" });
let log = "", browser;
server.stdout.on("data", (data) => { log += data; });
server.stderr.on("data", (data) => { log += data; });
try {
  const url = "http://localhost:5322";
  for (let i = 0; ; i++) {
    if (server.exitCode !== null || i === 120) throw new Error(log || "Vite did not start");
    try { if ((await fetch(url)).ok) break; } catch {}
    await sleep(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const [platform, width, height] of [["android", 390, 844], ["ios", 390, 844], ["android", 320, 480], ["desktop", 390, 844]]) {
    const label = `${platform}-${width}`;
    const page = await browser.newPage({ viewport: { width, height }, hasTouch: true });
    await page.addInitScript((platform) => {
      globalThis.__TINE_PLATFORM__ = platform;
      localStorage.setItem("logseq-claude.sidebarOpen", "0");
    }, platform);
    await page.goto(url);
    await page.locator("[data-search-trigger]").waitFor();
    await page.screenshot({ path: `${out}/${label}-navigation.png` });
    const search = page.locator("[data-search-trigger]");
    const rect = await search.boundingBox();
    assert.ok(rect && (platform === "desktop" ? rect.y < height / 2 : rect.y > height / 2), `${label}: search must be in the ${platform === "desktop" ? "top" : "bottom"} half; observed ${JSON.stringify(rect)}`);
    await search.click();
    await page.locator(".switcher-overlay").waitFor();
    await page.keyboard.press("Escape");
    console.log(`${label}: reachable search opens the page switcher`);
    if (platform !== "desktop") {
      // The transient onboarding toast can cover content on a short phone.
      // Dismiss it through its visible control before holding a page link.
      while (await page.locator(".toast-close").count()) await page.locator(".toast-close").first().tap();
      const link = page.locator(".main-content a.page-ref").filter({ hasText: "Tine" }).first();
      await link.scrollIntoViewIfNeeded();
      const box = await link.boundingBox();
      const cdp = await page.context().newCDPSession(page);
      await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: box.x + box.width / 2, y: box.y + box.height / 2 }] });
      await sleep(650);
      await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
      const preview = page.locator(".peek-popup");
      await preview.waitFor();
      const previewBox = await preview.boundingBox();
      assert.ok(previewBox.x >= 0 && previewBox.y >= 0 && previewBox.x + previewBox.width <= width && previewBox.y + previewBox.height <= height, `preview fits phone: ${JSON.stringify(previewBox)}`);
      await page.screenshot({ path: `${out}/${label}-preview.png` });
      // Tap the empty bottom padding, outside both the preview and controls.
      await page.touchscreen.tap(width - 5, height - 4);
      await preview.waitFor({ state: "detached" });
      await link.tap();
      await page.waitForFunction(async () => (await import("/src/router.ts")).route().name === "Tine");
      await page.locator('.mobile-navigation-toolbar [title="Go back"]').tap();
      await page.waitForFunction(async () => (await import("/src/router.ts")).route().kind === "journals");
      await page.locator('.mobile-navigation-toolbar [title="Go forward"]').tap();
      await page.waitForFunction(async () => (await import("/src/router.ts")).route().name === "Tine");
      await page.locator('.mobile-navigation-toolbar [title="Toggle sidebar (t l)"]').tap();
      await page.locator('[data-mobile-drawer-panel="left"]').waitFor();
      await page.keyboard.press("Escape");
      console.log(`${label}: real touch hold previews, release stays, outside tap dismisses, tap navigates, bottom Back/Forward and sidebar work`);
    }
    await page.close();
  }
} finally { await browser?.close(); server.kill(); }
