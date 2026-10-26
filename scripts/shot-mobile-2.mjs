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
  // GH #716: the top bar is the default on every mobile app; "Navigation buttons
  // at the bottom" restores the GH #598 row. Tablet width and landscape only
  // check placement (`quick`); the phone widths run the whole journey.
  const configs = [
    ["android", 390, 844, false], ["android", 390, 844, true],
    ["ios", 390, 844, false], ["ios", 390, 844, true],
    ["android", 320, 480, false], ["android", 320, 480, true],
    ["android", 844, 390, false, true], ["android", 844, 390, true, true],
    ["ios", 820, 1180, false, true], ["ios", 820, 1180, true, true],
    ["desktop", 390, 844, false],
  ];
  for (const [platform, width, height, bottom, quick] of configs) {
    const label = `${platform}-${width}x${height}-${bottom ? "bottom" : "top"}`;
    if (process.env.ONLY && !label.includes(process.env.ONLY)) continue;
    const page = await browser.newPage({ viewport: { width, height }, hasTouch: true });
    await page.addInitScript((platform) => {
      globalThis.__TINE_PLATFORM__ = platform;
      localStorage.setItem("logseq-claude.sidebarOpen", "0");
    }, platform);
    await page.goto(url);
    await page.locator("[data-search-trigger]").waitFor();
    if (bottom) {
      await page.evaluate(async () => (await import("/src/navSettings.ts")).setMobileNavigationAtBottom(true));
      await page.locator(".mobile-navigation-toolbar [data-search-trigger]").waitFor();
    }
    // Exactly one group, in the top bar by default and in the row when asked.
    const navRoot = bottom ? ".mobile-navigation-toolbar" : "header.topbar";
    assert.equal(await page.locator("[data-search-trigger]").count(), 1, `${label}: one navigation group`);
    assert.equal(await page.locator(`${navRoot} [data-search-trigger]`).count(), 1, `${label}: the group is in ${navRoot}`);
    assert.equal(await page.locator(".mobile-navigation-toolbar").count(), bottom ? 1 : 0, `${label}: bottom row only when the setting is on`);
    for (const name of ["Go back", "Go forward", "Toggle sidebar (t l)"]) {
      assert.equal(await page.locator(`${navRoot} [title="${name}"]`).count(), 1, `${label}: ${name} is in ${navRoot}`);
    }
    await page.screenshot({ path: `${out}/${label}-navigation.png` });
    const search = page.locator("[data-search-trigger]");
    const rect = await search.boundingBox();
    const wantTop = platform === "desktop" || !bottom;
    assert.ok(rect && (wantTop ? rect.y < height / 2 : rect.y > height / 2), `${label}: search must be in the ${wantTop ? "top" : "bottom"} half; observed ${JSON.stringify(rect)}`);
    await search.click();
    await page.locator(".switcher-overlay").waitFor();
    await page.keyboard.press("Escape");
    console.log(`${label}: reachable search opens the page switcher`);
    if (platform !== "desktop" && !quick) {
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
      // Tap outside both the preview and the controls: the empty bottom padding
      // (the bottom row's, or the page's end). A short phone's preview reaches
      // within touch-adjustment range of that padding, so there tap the empty
      // title-bar middle above it instead. With the group in the top bar the
      // bottom padding is page content, so the tap may start editing a block;
      // that is left again below.
      if (previewBox.y + previewBox.height < height - 24) await page.touchscreen.tap(width - 5, height - 4);
      else await page.touchscreen.tap(width / 2, previewBox.y / 2);
      await preview.waitFor({ state: "detached" });
      if (await page.locator("textarea.block-editor").count()) {
        await page.keyboard.press("Escape");
        await page.locator("textarea.block-editor").waitFor({ state: "detached" });
      }
      await link.tap();
      await page.waitForFunction(async () => (await import("/src/router.ts")).route().name === "Tine");
      // A 320px top bar (292px of content) folds Back/Forward into its "..." menu,
      // as stable does (GH #205); every wider width keeps them inline.
      const navTap = async (title, action) => {
        const inline = page.locator(`${navRoot} [title="${title}"]`);
        if (await inline.isVisible()) return inline.tap();
        await page.locator("[data-topbar-overflow-trigger]").tap();
        await page.locator(`[data-topbar-overflow-action="${action}"]`).tap();
      };
      await navTap("Go back", "back");
      await page.waitForFunction(async () => (await import("/src/router.ts")).route().kind === "journals");
      await navTap("Go forward", "forward");
      await page.waitForFunction(async () => (await import("/src/router.ts")).route().name === "Tine");
      await page.locator(`${navRoot} [title="Toggle sidebar (t l)"]`).tap();
      await page.locator('[data-mobile-drawer-panel="left"]').waitFor();
      await page.keyboard.press("Escape");
      console.log(`${label}: real touch hold previews, release stays, outside tap dismisses, tap navigates, ${bottom ? "bottom" : "top"} Back/Forward and sidebar work`);
    }
    if (platform === "android" && !quick && !bottom) {
      // GH #716 (default): the group is in the same place whether or not the
      // user is editing, with the keyboard up or after it closes.
      const ime = (visible) => page.evaluate((detail) => window.dispatchEvent(new CustomEvent("tine-ime", { detail })), visible);
      const at = async () => JSON.stringify(await page.locator("header.topbar [data-search-trigger]").boundingBox());
      const resting = await at();
      await page.locator(".main-content .ls-block .block-content").filter({ hasText: "Reads the same markdown graph", visible: true }).first().tap();
      await page.locator("textarea.block-editor").waitFor();
      await ime(true);
      await page.setViewportSize({ width, height: height - 300 });
      await page.locator("[data-mobile-keyboard-toolbar]").waitFor();
      assert.equal(await at(), resting, `${label}: the navigation group stays put while editing with the keyboard up`);
      await page.screenshot({ path: `${out}/${label}-keyboard-open.png` });
      const clear = await page.evaluate(() => { const r = document.querySelector("header.topbar [data-search-trigger]").getBoundingClientRect(); return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)?.closest("[data-search-trigger]") !== null; });
      assert.ok(clear, `${label}: nothing covers Search while editing`);
      await ime(false);
      await page.setViewportSize({ width, height });
      await page.locator("[data-mobile-keyboard-toolbar]").waitFor({ state: "hidden" });
      assert.equal(await at(), resting, `${label}: the navigation group stays put after the keyboard closes`);
      await page.screenshot({ path: `${out}/${label}-keyboard-closed.png` });
      console.log(`${label}: the top navigation group does not move when editing starts or the keyboard closes`);
    }
    if (platform === "android" && !quick && bottom) {
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
