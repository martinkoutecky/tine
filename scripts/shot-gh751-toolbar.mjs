// GH #751: actual browser shell, drawer, Block editor and toolbar. No native IME.
import { chromium } from "playwright";
import { createServer } from "vite";
import { mkdirSync, writeFileSync, realpathSync } from "node:fs";
import assert from "node:assert/strict";

const out = "artifacts/gh751-toolbar";
mkdirSync(out, { recursive: true });
const server = await createServer({ server: { port: 5351, strictPort: true, fs: { allow: [process.cwd(), realpathSync("node_modules"), realpathSync("node_modules/inter-ui")] } } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const platform of ["android", "ios"]) {
    const page = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true });
    await page.addInitScript(platform => {
      globalThis.__TINE_PLATFORM__ = platform;
      localStorage.setItem("logseq-claude.sidebarOpen", "0");
    }, platform);
    await page.goto("http://localhost:5351");
    await page.locator("[data-search-trigger]").waitFor();
    while (await page.locator(".toast-close").count()) await page.locator(".toast-close").first().tap();
    await page.evaluate(async () => (await import("/src/ui.ts")).openPageInSidebar("Tine"));
    const drawer = page.locator('[data-mobile-drawer-panel="right"]');
    await drawer.waitFor();
    const block = drawer.locator(".block-content").filter({ hasText: "Reads the same markdown graph", visible: true }).first();
    await block.tap();
    const editor = drawer.locator("textarea.block-editor");
    await editor.waitFor();
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("tine-ime", { detail: true })));
    if (platform === "android") await page.setViewportSize({ width: 390, height: 544 });
    else await page.evaluate(() => {
      // Overlay-keyboard geometry: the layout viewport/drawer stays tall.
      Object.defineProperty(window.visualViewport, "height", { configurable: true, value: 544 });
      window.dispatchEvent(new Event("resize"));
    });
    const toolbar = page.locator("[data-mobile-keyboard-toolbar]");
    await toolbar.waitFor();
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${out}/${platform}-drawer-editing.png` });
    const reachable = await toolbar.getByRole("button", { name: "TODO", exact: true }).evaluate(button => {
      const r = button.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return { reachable: hit === button || button.contains(hit), rect: r.toJSON(), hit: hit?.outerHTML.slice(0, 300), active: document.activeElement?.outerHTML.slice(0, 300) };
    });
    assert.ok(reachable.reachable, `${platform}: drawer must not cover the editing toolbar: ${JSON.stringify(reachable)}`);
    const before = await editor.inputValue();
    await toolbar.getByRole("button", { name: "TODO", exact: true }).tap();
    await page.waitForFunction(before => document.querySelector(".right-sidebar textarea.block-editor")?.value !== before, before);
    assert.match(await editor.inputValue(), /^TODO /, "toolbar action targets the sidebar editor");

    // At the very end, scrollTop must have enough range to reveal the editor,
    // even in a non-resizing visual viewport (iOS-style overlay keyboard).
    await page.evaluate(() => {
      const scroller = document.querySelector(".right-sidebar");
      const editor = scroller.querySelector("textarea.block-editor");
      const tail = document.createElement("div");
      tail.style.height = "900px";
      editor.closest(".rs-item").before(tail);
      scroller.scrollTop = 0;
      window.__mainScroll = document.querySelector(".main-content").scrollTop;
    });
    await page.evaluate(async () => (await import("/src/components/MobileKeyboardToolbar.tsx")).revealFocusedEditorAboveToolbar(document.querySelector("[data-mobile-keyboard-toolbar]").getBoundingClientRect().top));
    const reveal = await page.evaluate(() => ({
      bottom: document.querySelector(".right-sidebar textarea.block-editor").getBoundingClientRect().bottom,
      toolbarTop: document.querySelector("[data-mobile-keyboard-toolbar]").getBoundingClientRect().top,
      sidebarScroll: document.querySelector(".right-sidebar").scrollTop,
      mainUnchanged: document.querySelector(".main-content").scrollTop === window.__mainScroll,
    }));
    assert.ok(reveal.sidebarScroll > 0 && reveal.mainUnchanged && reveal.bottom <= reveal.toolbarTop - 7, JSON.stringify(reveal));

    // Force overlap to test actual hit-testing of every body-level popup family.
    const layers = await page.evaluate(() => {
      const toolbar = document.querySelector("[data-mobile-keyboard-toolbar]");
      const r = toolbar.getBoundingClientRect();
      return ["autocomplete", "peek-popup", "dp-overlay", "pp-overlay", "formula-editor-overlay", "rs-actions-menu rs-item-menu", "tab-overview", "switcher-overlay", "modal-overlay", "ctx-overlay", "audio-overlay", "lightbox-overlay"].map(className => {
        const popup = document.createElement("div");
        popup.className = className;
        Object.assign(popup.style, { position: "fixed", inset: "0", width: "auto", height: "auto", maxHeight: "none", margin: "0" });
        document.body.append(popup);
        const above = document.elementFromPoint(120, r.y + r.height / 2) === popup;
        popup.remove();
        return { className, above };
      });
    });
    assert.ok(layers.every(layer => layer.above), JSON.stringify(layers));
    writeFileSync(`${out}/${platform}-geometry.json`, JSON.stringify({ reachable, before, after: await editor.inputValue(), reveal, layers }, null, 2));
    await page.screenshot({ path: `${out}/${platform}-drawer-revealed.png` });
    await page.close();
    console.log(`${platform}: toolbar hit-test, focused action, drawer reveal and overlay order passed`);
  }
} finally {
  await browser?.close();
  await server.close();
}
