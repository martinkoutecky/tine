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
  let page = await browser.newPage({ viewport: { width: 1000, height: 800 }, permissions: ["clipboard-read", "clipboard-write"] });
  await page.goto(`http://localhost:${port}/scripts/fixtures/code-gutter.html`, { waitUntil: "domcontentloaded" });
  await page.locator(".code-gutter").waitFor();
  if (process.argv.includes("--labels")) {
    for (const width of [1000, 390]) {
      if (width === 390) {
        await page.close();
        page = await browser.newPage({ viewport: { width, height: 800 }, hasTouch: true, permissions: ["clipboard-read", "clipboard-write"] });
        await page.goto(`http://localhost:${port}/scripts/fixtures/code-gutter.html`, { waitUntil: "domcontentloaded" });
        await page.locator(".code-gutter").waitFor();
      }
      for (const lang of ["js", ""]) for (const kind of ["textarea", "cm6"]) for (const mixed of [false, true]) for (const editing of [false, true]) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(([kind, mixed, lang, editing]) => {
        window.showCode(2, kind, mixed, false, lang);
        if (!editing) window.finishCode();
      }, [kind, mixed, lang, editing]);
      const label = page.getByRole("button", { name: "Edit code language", exact: true });
      await label.waitFor();
      assert.equal(await label.textContent(), lang || "plain");
      if (editing) await page.waitForFunction(() => document.activeElement?.matches("textarea.block-editor, .cm-content"));
      if (editing) { if (width === 390) await label.tap(); else await label.click(); }
      else { await page.evaluate(() => window.selectCode()); await label.focus(); await page.keyboard.press(mixed ? "Space" : "Enter"); }
      await page.waitForFunction(() => window.codeState().value === window.codeState().raw);
      const state = await page.evaluate(() => window.codeState());
      assert.equal(state.caret, (mixed ? "Before\n".length : 0) + 3);
      await page.keyboard.type("text");
      const saved = await page.evaluate(() => window.finishCode());
      assert.equal(saved, state.raw.slice(0, state.caret) + "text" + state.raw.slice(state.caret));
      // Check Copy separately so focusing it cannot blur the editor before the label test.
      await page.evaluate(([kind, mixed, lang, editing]) => {
        window.showCode(2, kind, mixed, false, lang);
        if (!editing) window.finishCode();
      }, [kind, mixed, lang, editing]);
      const card = page.locator(".code-block, .code-editor-region").filter({ has: label });
      await card.hover();
      const copy = card.getByRole("button", { name: "Copy code", exact: true });
      if (editing) await page.waitForFunction(() => document.activeElement?.matches("textarea.block-editor, .cm-content"));
      else await copy.focus();
      const labelBox = await label.boundingBox(), copyBox = await copy.boundingBox();
      assert.ok(labelBox.x + labelBox.width <= copyBox.x, "label leaves Copy's hit target clear");
      if (width === 390) await copy.tap(); else await copy.click();
      assert.equal((await page.evaluate(() => navigator.clipboard.readText())).trimEnd(), "const n0 = 0;\nconst n1 = 1;");
      if (!editing && kind === "textarea" && !mixed) await page.screenshot({ path: `${out}/label-${width}-${lang || "plain"}.png` });
      results.push({ width, lang, kind, mixed, editing, caret: state.caret });
      }
    }
    writeFileSync(`${out}/labels.json`, JSON.stringify(results, null, 2) + "\n");
    console.log(`PASS: ${results.length} language label activations and source edits`);
  } else {
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
  }
} finally { await browser?.close(); server.kill(); }
