// GH #595: real Block on a routed synthetic page; Chromium paints actual CSS.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import assert from "node:assert/strict";
const port = 5314;
const out = process.env.OUT ?? "screenshots/qc4";
fs.mkdirSync(out, { recursive: true });
const server = spawn(process.execPath, ["./node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "pipe" });
let log = "", browser;
server.stdout.on("data", (chunk) => { log += chunk; });
server.stderr.on("data", (chunk) => { log += chunk; });
try {
  const url = `http://localhost:${port}/scripts/harness/qc4-editor.html`;
  for (let i = 0; ; i++) {
    if (server.exitCode !== null || i === 120) throw new Error(log || "Vite did not start");
    try { if ((await fetch(url)).ok) break; } catch {}
    await sleep(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const theme of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 900, height: 500 } });
    await page.goto(url);
    await page.waitForSelector(".block-content");
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, theme);
    // Reading state: a DOM text selection, including the browser's selection caret.
    await page.evaluate(() => {
      const el = document.querySelector(".block-content");
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
      let text; do { text = walker.nextNode(); } while (text && !text.textContent.includes("alpha selected omega"));
      const range = document.createRange(); range.setStart(text, 6); range.setEnd(text, 14);
      const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
    });
    assert.equal(await page.evaluate(() => getSelection().toString()), "selected");
    await page.screenshot({ path: `${out}/${theme}-reading.png` });
    await page.evaluate(() => getSelection().removeAllRanges());
    await page.locator(".block-content").click();
    const editor = page.locator("textarea.block-editor");
    await editor.waitFor();
    await editor.dblclick({ position: { x: 85, y: 15 } });
    const selected = () => editor.evaluate((el) => el.value.slice(el.selectionStart, el.selectionEnd));
    assert.equal(await selected(), "selected");
    await page.screenshot({ path: `${out}/${theme}-word.png` });
    const box = await editor.boundingBox();
    await page.mouse.move(box.x + 3, box.y + 15);
    await page.mouse.down(); await page.mouse.move(box.x + 130, box.y + 15, { steps: 12 }); await page.mouse.up();
    assert.ok((await selected()).includes("alpha selected"), "drag must select a phrase");
    await page.screenshot({ path: `${out}/${theme}-drag.png` });
    await editor.press("Control+a");
    assert.equal(await selected(), "alpha selected omega");
    await page.screenshot({ path: `${out}/${theme}-all.png` });
    const colors = await editor.evaluate((el) => ({
      selection: getComputedStyle(el, "::selection").backgroundColor,
      foreground: getComputedStyle(el).color,
      background: getComputedStyle(document.body).backgroundColor,
    }));
    assert.notEqual(colors.selection, "rgba(0, 0, 0, 0)");
    console.log(`${theme}: reading range, double-click word, dragged phrase, select-all; ${JSON.stringify(colors)}`);
    await page.close();
  }
  const mobile = await browser.newPage({ viewport: { width: 420, height: 600 } });
  await mobile.addInitScript(() => { globalThis.__TINE_PLATFORM__ = "android"; });
  await mobile.goto(url);
  await mobile.locator(".block-content").click();
  await mobile.locator("[data-mobile-keyboard-toolbar]:not([hidden])").waitFor();
  await mobile.locator('[aria-label="Redo"]').scrollIntoViewIfNeeded();
  await mobile.screenshot({ path: `${out}/android-toolbar.png` });
  console.log("Android toolbar: real focused Block and Undo/Redo icons captured");
} finally { await browser?.close(); server.kill(); }
