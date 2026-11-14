// Synthetic routed PageView; measure the painted number, its box and code separation.
// node scripts/shot-code-gutter.mjs [artifact-directory]
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";

const out = process.argv[2] ?? "target/artifacts/code-gutter";
mkdirSync(out, { recursive: true });
const port = 24579;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
const results = [];
try {
  for (let i = 0; ; i++) {
    if (server.exitCode !== null) throw new Error(`Vite exited ${server.exitCode}`);
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (i === 120) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1000, height: 800 } });
  await page.goto(`http://localhost:${port}/scripts/fixtures/code-gutter.html`, { waitUntil: "domcontentloaded" });
  await page.locator(".code-gutter").waitFor();
  if (process.argv[3]) await page.addStyleTag({ content: readFileSync(process.argv[3], "utf8") });
  for (const wrap of [false, true]) for (const kind of ["view", "textarea", "cm6"]) for (const mixed of [false, true]) for (const count of [9, 10, 99, 100, 1000]) {
    await page.evaluate(([n, k, m, w]) => window.showCode(n, k, m, w), [count, kind, mixed, wrap]);
    const last = page.locator(".code-gutter .calc-lineno").last();
    await last.waitFor();
    assert.equal(await last.textContent(), String(count));
    await last.scrollIntoViewIfNeeded();
    const geometry = await last.evaluate(el => {
      const box = el.getBoundingClientRect();
      const range = document.createRange(); range.selectNodeContents(el);
      const ink = range.getBoundingClientRect();
      const bands = [];
      for (const rect of range.getClientRects()) {
        if (rect.width && !bands.some(b => rect.top < b.bottom && rect.bottom > b.top)) bands.push(rect);
      }
      const rows = bands.length;
      const card = el.closest(".code-block, .code-editor-region");
      const gutter = el.closest(".code-gutter").getBoundingClientRect();
      const outer = (card.matches(".code-editor-region:not(.mixed-code-region)") ? card.parentElement : card).getBoundingClientRect();
      const editor = card.querySelector(".block-editor, .cm-content, code.hljs");
      const text = editor.getBoundingClientRect();
      const codeLeft = text.left + (editor.matches("textarea") ? parseFloat(getComputedStyle(editor).paddingLeft) : 0);
      return { number: el.textContent, rows, boxWidth: box.width, inkWidth: ink.width,
        inside: ink.left >= outer.left && ink.right <= outer.right,
        separate: ink.right < codeLeft, aligned: Math.abs(codeLeft - gutter.left) <= 1.5,
        codeLeft, numberRight: ink.right };
    });
    results.push({ wrap, kind, mixed, count, ...geometry });
    if ([10, 100, 1000].includes(count)) await page.screenshot({ path: `${out}/${wrap ? "wrap" : "nowrap"}-${kind}-${mixed ? "mixed" : "whole"}-${count}.png` });
  }
  writeFileSync(`${out}/geometry.json`, JSON.stringify(results, null, 2) + "\n");
  const failures = results.filter(r => r.rows !== 1 || r.inkWidth > r.boxWidth + .5 || !r.inside || !r.separate || !r.aligned);
  assert.deepEqual(failures, [], "each complete number fits its box inside the card, clear of the code");
  console.log(`PASS: ${results.length} code gutter layouts`);
  const numbered = [];
  for (const kind of ["textarea", "cm6"]) {
    await page.evaluate(k => window.showNumbered(k), kind);
    const editor = page.locator(kind === "cm6" ? ".cm-content" : "textarea.block-editor");
    await editor.waitFor(); await editor.focus();
    await page.keyboard.press("Control+Home");
    await page.keyboard.type("1. ");
    await page.keyboard.press("End"); await page.keyboard.press("Enter");
    await page.keyboard.type("Second line");
    await page.keyboard.press("Escape");
    await page.locator("li.md-list-item").waitFor();
    const numbers = await page.locator("li.md-list-item").evaluateAll(items => items.map(item => {
      // HTML's ordinal algorithm: an explicit value wins, otherwise count from ol.start.
      let number = item.parentElement.start;
      for (const sibling of item.parentElement.children) {
        if (sibling.hasAttribute("value")) number = sibling.value;
        if (sibling === item) return number;
        number++;
      }
    }));
    numbered.push({ kind, numbers });
    await page.screenshot({ path: `${out}/numbered-${kind}.png` });
  }
  writeFileSync(`${out}/numbered.json`, JSON.stringify(numbered, null, 2) + "\n");
  assert.deepEqual(numbered.map(r => r.numbers), [[2], [2]], "the continued line still displays 2 after editing");
  console.log("PASS: numbered continuation in both editors");
} finally { await browser?.close(); server.kill(); }
