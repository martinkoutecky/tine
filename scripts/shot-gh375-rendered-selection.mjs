// GH #375 layout probe in a mobile-UA Chromium (touch, coarse pointer): select
// rendered text while another block is edited with the "keyboard" up, then tap
// a formatting action. Prints the scroll position, edit state, toolbar box and
// selection at each step, and writes screenshots. Not a device proof: the IME
// is played by a viewport resize plus `tine-ime`, as MainActivity does.
// Usage: node scripts/shot-gh375-rendered-selection.mjs   (SHOT_DIR=... for the PNGs)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const OUT = process.env.SHOT_DIR || "/tmp/shot-gh375-rendered-selection";
mkdirSync(OUT, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("./node_modules/.bin/vite", ["--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
try {
  for (let a = 0; ; a++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch { /* not up yet */ }
    if (a === 160) throw new Error("vite did not start");
    await sleep(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const nav of [false, true]) {
    const ctx = await browser.newContext({ viewport: { width: 412, height: 900 }, hasTouch: true, isMobile: true,
      userAgent: "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36" });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => console.log("pageerror:", e.message));
    await page.goto(`http://localhost:${port}/scripts/fixtures/mobile-rendered-selection.html${nav ? "?nav" : ""}`);
    await page.locator('[data-block-id="b0"]').waitFor();
    const state = (label) => page.evaluate((label) => {
      const main = document.querySelector(".main-content");
      const bar = document.querySelector("[data-rendered-selection-toolbar]")?.getBoundingClientRect();
      const navRow = document.querySelector(".mobile-navigation-toolbar")?.getBoundingClientRect();
      return { label, scrollTop: Math.round(main.scrollTop), innerHeight, editing: window.__tine.editingId(),
        textarea: !!document.querySelector("textarea"), selection: getSelection().toString(),
        toolbar: bar ? { top: Math.round(bar.top), bottom: Math.round(bar.bottom) } : null,
        nav: navRow ? { top: Math.round(navRow.top), bottom: Math.round(navRow.bottom) } : null,
        b3: window.__tine.raw("b3") };
    }, label);
    // Editing far below with the keyboard up, page scrolled back to the top.
    await page.evaluate(() => { document.querySelector(".main-content").scrollTop = 99999; });
    await page.evaluate(() => window.__tine.startEditing("b25", 0, null));
    await sleep(300);
    await page.setViewportSize({ width: 412, height: 520 });
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("tine-ime", { detail: true })));
    await sleep(300);
    await page.evaluate(() => { document.querySelector(".main-content").scrollTop = 0; });
    await sleep(300);
    console.log(nav ? "[nav]" : "[no nav]", JSON.stringify(await state("editing b25, keyboard up")));
    // A long press selects "selectable" in b3 (Playwright has no native long-press selection).
    await page.evaluate(() => {
      const el = document.querySelector('[data-block-id="b3"] .block-content');
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
      let t = walker.nextNode();
      while (t && !t.textContent.includes("selectable")) t = walker.nextNode();
      const i = t.textContent.indexOf("selectable");
      getSelection().setBaseAndExtent(t, i, t, i + "selectable".length);
    });
    for (let i = 0; i < 4; i++) { console.log(nav ? "[nav]" : "[no nav]", JSON.stringify(await state(`selected +${i * 100}ms`))); await sleep(100); }
    // Ending the edit closes the keyboard on a device; play that.
    await page.setViewportSize({ width: 412, height: 900 });
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("tine-ime", { detail: false })));
    await sleep(300);
    console.log(nav ? "[nav]" : "[no nav]", JSON.stringify(await state("keyboard closed")));
    await page.screenshot({ path: `${OUT}/${nav ? "nav" : "nonav"}-selected.png` });
    await page.locator('[data-rendered-selection-toolbar] [data-selection-action="bold"]').tap();
    await sleep(300);
    console.log(nav ? "[nav]" : "[no nav]", JSON.stringify(await state("tapped Bold")));
    await page.screenshot({ path: `${OUT}/${nav ? "nav" : "nonav"}-bold.png` });
    await ctx.close();
  }
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch { /* gone */ }
}
