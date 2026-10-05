// Production Chromium interaction/geometry proof at real App entry points.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import assert from "node:assert/strict";
const output = "test-results/qf1/interactions";
mkdirSync(output, { recursive: true });
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", "5272", "--strictPort"], { stdio: "ignore" });
let browser, page;
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch("http://localhost:5272")).ok) break; } catch {}
    if (i > 100) throw new Error("preview did not start");
    await delay(100);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto("http://localhost:5272/?big");
  await page.waitForSelector(".page-title");
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Big");
  await page.waitForFunction(() => document.querySelector(".switcher-name")?.textContent.trim() === "Big");
  await page.locator(".switcher-row").first().click();
  await page.waitForFunction(() => document.querySelector(".page-title")?.textContent.trim() === "Big");
  assert.ok(await page.locator(".ls-block").count() < 200);
  const find = async (text) => {
    await page.keyboard.press("Control+f");
    await page.locator(".inpage-find-input").fill(text);
    await page.waitForFunction((text) => {
      const block = document.querySelector(".inpage-find-active-block");
      const sc = document.querySelector(".main-content");
      if (!block?.textContent.includes(text) || !sc) return false;
      const r = block.getBoundingClientRect(), v = sc.getBoundingClientRect();
      return r.top >= v.top && r.top < v.bottom;
    }, text);
    const id = await page.locator(".inpage-find-active-block").getAttribute("data-block-id");
    await page.keyboard.press("Escape");
    return id;
  };
  const farId = await find("Block 1799:");
  await page.locator(`[data-block-id="${farId}"] .block-content`).click({ position: { x: 10, y: 12 } });
  const editor = page.locator("textarea.block-editor");
  await editor.press("End"); await editor.press("ArrowDown");
  await page.waitForFunction(() => document.querySelector("textarea.block-editor")?.value.startsWith("Paragraph **1800**"));
  await editor.press("Home"); await editor.press("ArrowUp");
  await page.waitForFunction((id) => document.querySelector("textarea.block-editor")?.closest(".ls-block").dataset.blockId === id, farId);
  await editor.fill("Saved far edit for window history"); await editor.press("Escape");
  const offset = await page.locator(".main-content").evaluate((e) => e.scrollTop);
  await page.locator(".nav-item", { hasText: "Journals" }).first().click();
  await page.waitForFunction(() => document.querySelector(".page-title")?.textContent.trim() !== "Big");
  await page.keyboard.press("Alt+ArrowLeft");
  await page.waitForFunction(({ id, offset }) => {
    const sc = document.querySelector(".main-content"), row = document.querySelector(`[data-block-id="${id}"]`);
    return row?.textContent.includes("Saved far edit") && Math.abs(sc.scrollTop - offset) < 100;
  }, { id: farId, offset });
  // Return to the earlier journal entry, then Forward must restore this page.
  await page.keyboard.press("Alt+ArrowLeft");
  await page.waitForFunction(() => document.querySelector(".page-title")?.textContent.trim() !== "Big");
  await page.keyboard.press("Alt+ArrowRight");
  await page.waitForFunction(({ id, offset }) => {
    const sc = document.querySelector(".main-content"), row = document.querySelector(`[data-block-id="${id}"]`);
    return row?.textContent.includes("Saved far edit") && Math.abs(sc.scrollTop - offset) < 100;
  }, { id: farId, offset });
  await page.screenshot({ path: `${output}/back.png` });
  await page.locator(`[data-block-id="${farId}"] .block-content`).click({ position: { x: 10, y: 12 } });
  await editor.press("Escape"); await page.keyboard.press("Shift+ArrowDown");
  await page.waitForFunction(() => [...document.querySelectorAll(".block-main.selected")].some((e) => e.textContent.includes("Paragraph 1800")));
  assert.ok(await page.locator(".block-main.selected").count() >= 2);

  // Visit top/bottom twice: already-measured windows must retain the same extent.
  const scroll = async (bottom) => {
    await page.locator(".main-content").evaluate((e, bottom) => { e.scrollTop = bottom ? e.scrollHeight : 0; }, bottom);
    await delay(500); // visual observer/layout settling, never save readiness
    return page.locator(".main-content").evaluate((e) => e.scrollHeight);
  };
  await scroll(false); await scroll(true); await scroll(false);
  const stable = await scroll(true);
  await scroll(false);
  const again = await scroll(true);
  assert.equal(again, stable, "seen windows changed scrollbar extent on re-entry");
  await scroll(false);
  const source = page.locator(".ls-block").first();
  const sourceId = await source.getAttribute("data-block-id");
  await source.locator(".block-content").click({ position: { x: 10, y: 12 } }); await editor.press("Escape");
  const bullet = await source.locator(".bullet-container").boundingBox();
  await page.mouse.move(bullet.x + bullet.width / 2, bullet.y + bullet.height / 2);
  await page.mouse.down(); await page.mouse.move(bullet.x + bullet.width / 2 + 10, bullet.y + bullet.height / 2 + 10);
  // Dispatch the pointer move in the same tick as scrolling, before IO can mount
  // the destination. The viewport's capture handler must warm the drop target.
  const target = await page.evaluate((offset) => {
    const sc = document.querySelector(".main-content"); sc.scrollTop = offset;
    const rect = sc.getBoundingClientRect();
    const x = document.querySelector(".page-blocks").getBoundingClientRect().left + 25, y = rect.top + 300;
    const el = document.elementFromPoint(x, y);
    const wasUnrendered = !!el?.closest("[data-outline-window]") && !el?.closest(".ls-block");
    el?.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y, bubbles: true, buttons: 1 }));
    const indicated = document.querySelector(".drop-before,.drop-after,.drop-child");
    return { wasUnrendered, id: indicated?.closest(".ls-block").dataset.blockId };
  }, offset);
  assert.ok(target.wasUnrendered, "drop fixture was already rendered");
  assert.ok(target.id, "unrendered position did not become a drop target");
  await page.mouse.up();
  await find("Paragraph 0 with emphasis");
  assert.ok(await page.locator(".main-content").evaluate((e) => e.scrollTop) > offset / 2, "dragged block did not move to the far-down destination");
  assert.ok(await page.locator(`[data-block-id="${sourceId}"]`).count());
  await page.screenshot({ path: `${output}/drop.png` });
  assert.deepEqual(errors, []);
  console.log("PASS: far Find; Up/Down across window edge; saved Back/Forward; Shift extension; stable seen extent; drag to unrendered position");
} catch (error) {
  await page?.screenshot({ path: `${output}/failure.png` });
  console.log(await page?.evaluate(() => ({ title: document.querySelector(".page-title")?.textContent, editors: document.querySelectorAll("textarea").length, shells: document.querySelectorAll(".ls-block").length, active: document.activeElement?.tagName })));
  throw error;
} finally {
  await browser?.close(); server.kill("SIGKILL");
}
