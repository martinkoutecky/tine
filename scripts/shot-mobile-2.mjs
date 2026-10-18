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
      // Sticky-parent copies are zero-size duplicates, and a short phone mounts
      // only the blocks near the viewport: scroll until the real link exists.
      const link = page.locator(".main-content a.page-ref").filter({ hasText: "Tine", visible: true }).first();
      for (let i = 0; !(await link.count()) && i < 40; i++) {
        await page.locator(".main-content").evaluate((el) => el.scrollBy(0, 200));
        await sleep(100);
      }
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
      // A short phone's preview reaches within touch-adjustment range of that
      // padding, so there tap the empty title-bar middle above it instead.
      if (previewBox.y + previewBox.height < height - 24) await page.touchscreen.tap(width - 5, height - 4);
      else await page.touchscreen.tap(width / 2, previewBox.y / 2);
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
    if (platform === "android") {
      // GH #716: Back (or the IME's own hide key) closes the keyboard but keeps
      // the block editor focused. Android pads the WebView above the IME, which
      // a viewport resize reproduces, and MainActivity reports the IME as a
      // `tine-ime` event, sent here by hand. The bottom row must come back
      // unobscured and tappable, not stay replaced by the editing dock. iOS
      // shrinks only the visual viewport, which Chromium cannot reproduce; the
      // unit tests cover it.
      const ime = (visible) => page.evaluate((detail) => window.dispatchEvent(new CustomEvent("tine-ime", { detail })), visible);
      // A plain-text block on the routed Tine page (no link under the tap).
      await page.locator(".main-content .ls-block .block-content").filter({ hasText: "Reads the same markdown graph", visible: true }).first().tap();
      await page.locator("textarea.block-editor").waitFor();
      const dock = page.locator("[data-mobile-keyboard-toolbar]");
      await ime(true);
      await page.setViewportSize({ width, height: height - 300 });
      await dock.waitFor();
      await page.screenshot({ path: `${out}/${label}-keyboard-open.png` });
      await ime(false);
      await page.setViewportSize({ width, height });
      await dock.waitFor({ state: "hidden" });
      assert.ok(await page.locator("textarea.block-editor").evaluate((el) => el === document.activeElement), `${label}: the editor keeps focus after the keyboard closes`);
      await page.screenshot({ path: `${out}/${label}-keyboard-closed.png` });
      const bottomSearch = page.locator(".mobile-navigation-toolbar [data-search-trigger]");
      const searchBox = await bottomSearch.boundingBox();
      assert.ok(searchBox && searchBox.y + searchBox.height <= height, `${label}: bottom Search is on screen after the keyboard closes; observed ${JSON.stringify(searchBox)}`);
      const hit = await page.evaluate(([x, y]) => document.elementFromPoint(x, y)?.closest("[data-search-trigger]") !== null, [searchBox.x + searchBox.width / 2, searchBox.y + searchBox.height / 2]);
      assert.ok(hit, `${label}: nothing covers the bottom Search after the keyboard closes`);
      // Bottom-anchored chrome returns to rest above the row, not to a lift
      // measured from the hidden dock.
      const help = await page.locator(".help-corner-btn").boundingBox();
      assert.ok(help && help.y > height / 2 && help.y + help.height <= searchBox.y, `${label}: the help button rests just above the bottom row; observed ${JSON.stringify(help)}`);
      await bottomSearch.tap();
      await page.locator(".switcher-overlay").waitFor();
      await page.keyboard.press("Escape");
      console.log(`${label}: after the keyboard closes with the editor focused, the bottom row is unobscured and Search opens`);
    }
    await page.close();
  }
} finally { await browser?.close(); server.kill(); }
